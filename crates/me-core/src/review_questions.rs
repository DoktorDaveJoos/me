//! Unverified suggestions require an explicit user answer or selected batch approval.
//! Their model-written quotations must never become document evidence.
use crate::vault::{decision, hash, id, journal, segment, sql_id};
use crate::{Error, ExtractedFact, RejectedFact, Result, STANDARD_FIELDS, Vault, normalize_fact};
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::{Value, json};

#[derive(Clone, Debug)]
pub struct ReviewQuestion {
    pub id: String,
    pub label: String,
    pub value: String,
    pub claimed_quote: String,
    pub claimed_subject: String,
    pub reason: String,
    pub source_excerpt: Option<String>,
    pub location_label: Option<String>,
    pub existing_values: Vec<String>,
}

pub fn question_warning(count: usize) -> Option<String> {
    match count {
        0 => None,
        1 => Some("1 detail needs confirmation in Review.".into()),
        n => Some(format!("{n} details need confirmation in Review.")),
    }
}

pub(crate) fn candidate_key(fact: &ExtractedFact) -> String {
    hash(
        json!([
            fact.property,
            normalize_fact(&fact.property, &fact.value).unwrap_or_else(|_| fact.value.clone()),
            fact.context_quote,
            if crate::document_field_label(&fact.property).is_some()
                && fact.context_quote.is_empty()
            {
                &fact.segment_id
            } else {
                ""
            },
            fact.subject_quote
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        ])
        .to_string()
        .as_bytes(),
    )
}

pub(crate) fn store_questions(
    tx: &Transaction<'_>,
    source: &str,
    run: &str,
    questions: Vec<RejectedFact>,
) -> Result<()> {
    for question in questions {
        let fact = question.fact;
        if (!STANDARD_FIELDS.iter().any(|f| f.key == fact.property)
            && crate::document_field_label(&fact.property).is_none())
            || fact.value.len() > 16000
            || fact.quote.len() > 64000
            || fact.subject_quote.len() > 16000
        {
            return Err(Error::Validation(
                "The suggested detail is invalid. Try analyzing again.",
            ));
        }
        let property = crate::extraction_fields::ensure_extraction_property(tx, source, &fact)?;
        // A model's segment id is only a hint. Resolve it against this source.
        let segment: Option<String> = tx
            .query_row(
                "SELECT id FROM source_segment WHERE id=? AND source_id=? AND ordinal>0",
                params![fact.segment_id, source],
                |r| r.get(0),
            )
            .optional()?;
        tx.execute("INSERT OR IGNORE INTO ai_question(id,source_id,candidate_key,property_key,value,claimed_quote,claimed_subject,segment_id,reason_code,run_id,state) VALUES(?,?,?,?,?,?,?,?,?,?,'pending')", params![id(),source,candidate_key(&fact),property,fact.value,fact.quote,fact.subject_quote,segment,question.code,run])?;
    }
    Ok(())
}

pub(crate) fn refresh_review_state(tx: &Transaction<'_>, source: &str) -> Result<()> {
    let count: i64 = tx.query_row(
        "SELECT count(*) FROM ai_question WHERE source_id=? AND state='pending'",
        [source],
        |r| r.get(0),
    )?;
    let proposals: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM ai_proposal WHERE source_id=? AND state='proposed')",
        [source],
        |r| r.get(0),
    )?;
    tx.execute(
        "UPDATE document_evaluation SET warning_message=? WHERE source_id=?",
        params![question_warning(count as usize), source],
    )?;
    tx.execute(
        "UPDATE job SET state=? WHERE source_id=? AND kind='extract_facts' AND state!='running'",
        params![
            if count > 0 || proposals {
                "needs_review"
            } else {
                "done"
            },
            source
        ],
    )?;
    Ok(())
}

fn reason(code: &str) -> &'static str {
    match code {
        "unknown_segment" => "The suggested source couldn't be found.",
        "quote_not_in_segment" => "The suggested quotation wasn't found in the source.",
        "value_not_in_quote" => "The source doesn't clearly confirm this value.",
        "context_not_in_source" => "The period or context couldn't be verified in the document.",
        "subject_not_in_source" => "The suggested person couldn't be verified in this document.",
        _ => "The source couldn't be verified.",
    }
}

impl Vault {
    pub fn review_questions(&self, item: u64) -> Result<Vec<ReviewQuestion>> {
        let mut stmt = self.db.prepare("SELECT q.id,d.label,q.value,q.claimed_quote,q.claimed_subject,q.reason_code,g.text,g.locator_json,q.property_key FROM ai_question q JOIN property_definition d ON d.key=q.property_key JOIN collection_item i ON i.source_id=q.source_id JOIN source s ON s.id=q.source_id LEFT JOIN source_segment g ON g.id=q.segment_id AND g.source_id=q.source_id WHERE i.local_id=? AND i.deleted_at IS NULL AND s.sensitivity='personal' AND s.retention='keep' AND q.state='pending' ORDER BY q.property_key,q.rowid")?;
        let rows = stmt
            .query_map([sql_id(item)?], |r| {
                let locator: Option<String> = r.get(7)?;
                let locator: Value = locator
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or(Value::Null);
                Ok((
                    ReviewQuestion {
                        id: r.get(0)?,
                        label: r.get(1)?,
                        value: r.get(2)?,
                        claimed_quote: r.get(3)?,
                        claimed_subject: r.get(4)?,
                        reason: reason(&r.get::<_, String>(5)?).into(),
                        source_excerpt: r.get(6)?,
                        location_label: locator["page"]
                            .as_u64()
                            .map(|p| format!("Page {p}"))
                            .or_else(|| locator["section"].as_str().map(str::to_owned)),
                        existing_values: Vec::new(),
                    },
                    r.get::<_, String>(8)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter().map(|(mut question,property)| {
            let mut stmt=self.db.prepare("SELECT DISTINCT value_json FROM assertion_state WHERE state='accept' AND subject_id=(SELECT profile_id FROM vault_meta) AND property_key=? LIMIT 20")?;
            question.existing_values=stmt.query_map([property],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?.iter().filter_map(|s|serde_json::from_str(s).ok()).collect();
            Ok(question)
        }).collect()
    }

    /// Local review UI only. Some(value) explicitly confirms ownership and the
    /// (possibly edited) value; None dismisses it. Never exposed as an AI tool.
    pub fn answer_review_question(
        &mut self,
        item: u64,
        question: &str,
        answer: Option<&str>,
    ) -> Result<()> {
        let tx = self.db.transaction()?;
        answer_question_tx(&tx, item, question, answer)?;
        tx.commit()?;
        Ok(())
    }
}

pub(crate) fn answer_question_tx(
    tx: &Transaction<'_>,
    item: u64,
    question: &str,
    answer: Option<&str>,
) -> Result<()> {
    let row: Option<(String,String)> = tx.query_row("SELECT q.source_id,q.property_key FROM ai_question q JOIN collection_item i ON i.source_id=q.source_id JOIN source s ON s.id=q.source_id WHERE q.id=? AND i.local_id=? AND i.deleted_at IS NULL AND q.state='pending' AND s.sensitivity='personal' AND s.retention='keep' AND NOT EXISTS(SELECT 1 FROM extraction_run e WHERE e.source_id=s.id AND e.status='running')",params![question,sql_id(item)?],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    let (document, property) = row.ok_or(Error::Validation(
        "This detail is no longer pending or is still being analyzed.",
    ))?;
    let value = answer
        .map(|value| normalize_fact(&property, value))
        .transpose()?;
    if let Some(value) = &value {
        let (label, value_type): (String, String) = tx.query_row(
            "SELECT label,value_type FROM property_definition WHERE key=?",
            [&property],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let profile: String =
            tx.query_row("SELECT profile_id FROM vault_meta", [], |r| r.get(0))?;
        let source = id();
        // A new user-authored source is the evidence; the PDF remains only
        // a contextual link. Do not attach the unsupported model quotation.
        tx.execute("INSERT INTO source(id,kind,title,received_at,content_fingerprint,sensitivity,retention) VALUES(?,'note',?,strftime('%Y-%m-%dT%H:%M:%fZ','now'),?,'personal','keep')",params![source,label,hash(value.as_bytes())])?;
        let raw = json!(value).to_string();
        let semantic = hash(
            json!([profile, property, value, "unknown"])
                .to_string()
                .as_bytes(),
        );
        let assertion = tx
            .query_row(
                "SELECT id FROM assertion WHERE semantic_key=?",
                [&semantic],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .unwrap_or_else(id);
        tx.execute("INSERT OR IGNORE INTO assertion(id,subject_id,property_key,value_type,value_json,canonical_value,semantic_key,time_kind,recorded_at,origin) VALUES(?,?,?,?,?,?,?,'unknown',strftime('%Y-%m-%dT%H:%M:%fZ','now'),'user')",params![assertion,profile,property,value_type,raw,raw,semantic])?;
        decision(tx, &assertion, "accept")?;
        let locator = json!({"kind":"manual_confirmation","review_question_id":question,"context_source_id":document});
        tx.execute(
            "INSERT INTO assertion_evidence VALUES(?,?,?,?)",
            params![assertion, source, id(), locator.to_string()],
        )?;
        tx.execute(
            "INSERT INTO source_entity VALUES(?,?,'provided_by','accepted')",
            params![source, profile],
        )?;
        segment(tx, &source, 0, &format!("{}\n{value}", label))?;
        let already_listed:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM collection_item WHERE current_assertion_id=? AND deleted_at IS NULL)",[&assertion],|r|r.get(0))?;
        if !already_listed {
            let stable = id();
            tx.execute("INSERT INTO collection_item(stable_id,title,kind,source_id,property_key,current_assertion_id) VALUES(?,?,'note',?,?,?)",params![stable,label,source,property,assertion])?;
            journal(
                tx,
                &stable,
                1,
                "review_answered",
                json!({"assertion_id":assertion,"source_id":source}),
            )?;
        }
    }
    tx.execute(
        "UPDATE ai_question SET state=?,answer_value=? WHERE id=?",
        params![
            if value.is_some() {
                "confirmed"
            } else {
                "dismissed"
            },
            value,
            question
        ],
    )?;
    refresh_review_state(tx, &document)?;
    Ok(())
}
