use crate::{
    Error, Result, Vault,
    vault::{decision, hash, id, journal},
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Serialize)]
pub struct StandardField {
    pub key: &'static str,
    pub label: &'static str,
    pub value_type: &'static str,
}
pub const STANDARD_FIELDS: [StandardField; 3] = [
    StandardField {
        key: "person.tax_id",
        label: "Tax ID",
        value_type: "identifier",
    },
    StandardField {
        key: "person.social_insurance_number",
        label: "Social insurance number",
        value_type: "identifier",
    },
    StandardField {
        key: "person.birth_date",
        label: "Date of birth",
        value_type: "date",
    },
];
pub fn standard_field(label: &str) -> Option<&'static StandardField> {
    let label: String = label
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    match label.as_str() {
        "taxid" | "taxidentificationnumber" | "steuerid" | "steueridentifikationsnummer" => {
            Some(&STANDARD_FIELDS[0])
        }
        "socialinsurancenumber"
        | "sozialversicherungsnummer"
        | "svnummer"
        | "rentenversicherungsnummer" => Some(&STANDARD_FIELDS[1]),
        "dateofbirth" | "birthdate" | "geburtsdatum" => Some(&STANDARD_FIELDS[2]),
        _ => None,
    }
}
pub fn normalize_fact(key: &str, value: &str) -> Result<String> {
    let compact: String = value.chars().filter(|c| !c.is_whitespace()).collect();
    match key {
        "person.tax_id" if compact.len() == 11 && compact.bytes().all(|b| b.is_ascii_digit()) => {
            Ok(compact)
        }
        "person.social_insurance_number" => {
            let value = compact.to_ascii_uppercase();
            if value.len() == 12
                && value.bytes().enumerate().all(|(i, b)| {
                    if i == 8 {
                        b.is_ascii_uppercase()
                    } else {
                        b.is_ascii_digit()
                    }
                })
            {
                Ok(value)
            } else {
                Err(Error::Validation(
                    "A German social insurance number needs eight digits, one letter, and three digits.",
                ))
            }
        }
        "person.birth_date" => {
            let (year, month, day) = if compact.len() == 10
                && compact.is_ascii()
                && &compact[4..5] == "-"
                && &compact[7..8] == "-"
            {
                (&compact[0..4], &compact[5..7], &compact[8..10])
            } else {
                let parts: Vec<_> = compact.split('.').collect();
                if parts.len() != 3
                    || !(1..=2).contains(&parts[0].len())
                    || !(1..=2).contains(&parts[1].len())
                    || parts[2].len() != 4
                {
                    return Err(Error::Validation(
                        "Enter a birth date as DD.MM.YYYY or YYYY-MM-DD.",
                    ));
                }
                (parts[2], parts[1], parts[0])
            };
            if ![year, month, day]
                .iter()
                .all(|s| s.bytes().all(|b| b.is_ascii_digit()))
            {
                return Err(Error::Validation("Invalid date."));
            }
            let y: u32 = year.parse().map_err(|_| Error::Format)?;
            let m: u32 = month.parse().map_err(|_| Error::Format)?;
            let d: u32 = day.parse().map_err(|_| Error::Format)?;
            let days = match m {
                4 | 6 | 9 | 11 => 30,
                2 if y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400)) => 29,
                2 => 28,
                1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
                _ => 0,
            };
            if y == 0 || d == 0 || d > days {
                return Err(Error::Validation("Invalid date."));
            }
            Ok(format!("{y:04}-{m:02}-{d:02}"))
        }
        "person.tax_id" => Err(Error::Validation(
            "A German tax ID needs eleven digits. A tax office number is a different field.",
        )),
        _ if crate::document_field_label(key).is_some() => {
            let value = value.trim();
            if value.is_empty()
                || value.len() > 4000
                || value.chars().any(|c| c.is_control() && !c.is_whitespace())
            {
                return Err(Error::Validation("Invalid document value."));
            }
            Ok(value.split_whitespace().collect::<Vec<_>>().join(" "))
        }
        _ => Err(Error::Validation("Unknown standard field.")),
    }
}

/// A snapshot created only by the trusted desktop UI, never from tool arguments.
#[derive(Clone, Default)]
pub struct ReadScope {
    sources: Vec<String>,
}
impl ReadScope {
    fn json(&self) -> String {
        json!(self.sources).to_string()
    }
    pub fn len(&self) -> usize {
        self.sources.len()
    }
    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedFact {
    pub property: String,
    pub value: String,
    pub segment_id: String,
    pub quote: String,
    /// Exact associated party from the source; empty routes the value to ownership review.
    pub subject_quote: String,
    /// Exact period/account/section wording from this source; empty if unavailable.
    #[serde(default)]
    pub context_quote: String,
}
/// Overall bound across all bounded model calls for one document.
pub const MAX_EXTRACTION_FACTS: usize = 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractionOutput {
    pub facts: Vec<ExtractedFact>,
}
#[derive(Clone, Debug, Serialize)]
pub struct SourceText {
    pub segment_id: String,
    pub ordinal: i64,
    pub text: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct ExtractionInput {
    pub run_id: String,
    pub source_id: String,
    pub title: String,
    pub segments: Vec<SourceText>,
}
impl ExtractionInput {
    pub fn with_segments(&self, segments: Vec<SourceText>) -> Self {
        Self {
            run_id: self.run_id.clone(),
            source_id: self.source_id.clone(),
            title: self.title.clone(),
            segments,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Proposal {
    pub id: String,
    pub label: String,
    pub value: String,
    pub quote: String,
    pub subject_quote: String,
    pub existing_values: Vec<String>,
    pub location_label: String,
}

impl Vault {
    pub fn shareable_scope(&self) -> Result<ReadScope> {
        let mut stmt = self.db.prepare("SELECT DISTINCT s.id FROM source s JOIN collection_item i ON i.source_id=s.id WHERE s.sensitivity='personal' AND s.retention='keep' AND i.deleted_at IS NULL")?;
        let sources = stmt
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<Vec<String>, _>>()?;
        Ok(ReadScope { sources })
    }

    pub fn facts_get(&self, scope: &ReadScope, fields: &[String]) -> Result<Value> {
        if fields.is_empty() || fields.len() > 16 {
            return Err(Error::Validation("Request between 1 and 16 fields."));
        }
        let mut facts = Vec::new();
        for field in fields {
            if !STANDARD_FIELDS.iter().any(|f| f.key == field) {
                return Err(Error::Validation(
                    "Unknown field. Use field keys from me_status.",
                ));
            }
            let mut stmt = self.db.prepare("SELECT DISTINCT a.id,a.value_json,s.id,s.title,e.locator_json FROM assertion_state a JOIN assertion_evidence e ON e.assertion_id=a.id JOIN source s ON s.id=e.source_id WHERE a.subject_id=(SELECT profile_id FROM vault_meta) AND a.property_key=? AND a.state='accept' AND s.sensitivity='personal' AND s.retention='keep' AND s.id IN (SELECT value FROM json_each(?)) ORDER BY a.id,s.id LIMIT 11")?;
            let rows = stmt
                .query_map(params![field, scope.json()], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let overflow = rows.len() > 10;
            let mut candidates = Vec::new();
            let mut accepted = std::collections::BTreeSet::new();
            for (assertion, raw, source, title, locator) in rows.into_iter().take(10) {
                let value: String = serde_json::from_str(&raw).map_err(|_| Error::Format)?;
                accepted.insert(value.clone());
                candidates.push(json!({"assertion_id":assertion,"value":value,"state":"accepted","source_id":source,"source_title":title,"locator":serde_json::from_str::<Value>(&locator).map_err(|_| Error::Format)?}));
            }
            let mut stmt = self.db.prepare("SELECT p.id,p.value,p.source_id,s.title,p.quote FROM ai_proposal p JOIN source s ON s.id=p.source_id WHERE p.property_key=? AND p.state='proposed' AND s.sensitivity='personal' AND s.retention='keep' AND s.id IN (SELECT value FROM json_each(?)) LIMIT 11")?;
            let pending = stmt
                .query_map(params![field, scope.json()], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let mut distinct = accepted.clone();
            let overflow = overflow || pending.len() > 10;
            let has_pending = !pending.is_empty();
            for (id, value, source, title, quote) in pending.into_iter().take(10) {
                distinct.insert(value.clone());
                candidates.push(json!({"proposal_id":id,"value":value,"state":"unverified","subject":"unverified","source_id":source,"source_title":title,"quote":quote}));
            }
            // Even an equal pending value can belong to another person. Never
            // promote it; accepted values remain separately labelled.
            let status = if overflow {
                "incomplete"
            } else if distinct.len() > 1 {
                "conflicting"
            } else if accepted.len() == 1 {
                "resolved"
            } else if has_pending {
                "unverified"
            } else {
                "missing"
            };
            let value = if status == "resolved" {
                accepted.first().cloned()
            } else {
                None
            };
            facts.push(
                json!({"property":field,"status":status,"value":value,"candidates":candidates}),
            );
        }
        Ok(json!({"subject":"self","coverage":"shared_sources_only","facts":facts}))
    }

    pub fn documents_search(&self, scope: &ReadScope, query: &str, limit: usize) -> Result<Value> {
        if query.len() > 1024 || !(1..=30).contains(&limit) {
            return Err(Error::Validation("Invalid search query or result limit."));
        }
        let words: Vec<_> = query
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .take(16)
            .collect();
        if words.is_empty() {
            return Err(Error::Validation("Enter a search term."));
        }
        let fts = words
            .iter()
            .map(|s| format!("\"{s}\"*"))
            .collect::<Vec<_>>()
            .join(" AND ");
        let mut stmt = self.db.prepare("SELECT g.id,s.id,s.title,g.locator_json,g.text FROM segment_fts JOIN source_segment g ON g.rowid=segment_fts.rowid JOIN source s ON s.id=g.source_id WHERE segment_fts MATCH ? AND s.sensitivity='personal' AND s.retention='keep' AND s.id IN (SELECT value FROM json_each(?)) ORDER BY rank LIMIT ?")?;
        let rows = stmt
            .query_map(params![fts, scope.json(), (limit + 1) as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let more = rows.len() > limit;
        let hits: Vec<_> = rows.into_iter().take(limit).map(|(segment,source,title,locator,text)| json!({"segment_id":segment,"source_id":source,"title":title,"locator":serde_json::from_str::<Value>(&locator).unwrap_or(Value::Null),"excerpt":text.chars().take(500).collect::<String>()})).collect();
        Ok(json!({"hits":hits,"has_more":more,"coverage":"shared_sources_only"}))
    }

    pub fn evidence_read(&self, scope: &ReadScope, segment: &str) -> Result<Value> {
        if segment.len() > 80 {
            return Err(Error::Validation("Invalid source reference."));
        }
        let row = self.db.query_row("SELECT s.id,s.title,g.text,g.locator_json FROM source_segment g JOIN source s ON s.id=g.source_id WHERE g.id=? AND s.sensitivity='personal' AND s.retention='keep' AND s.id IN (SELECT value FROM json_each(?))", params![segment,scope.json()], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?))).optional()?;
        let (source, title, text, locator) = row.ok_or(Error::Validation(
            "The source is unavailable or hasn't been released.",
        ))?;
        if text.len() > 18000 {
            return Err(Error::Validation("This source excerpt is too large."));
        }
        Ok(
            json!({"segment_id":segment,"source_id":source,"title":title,"text":text,"locator":serde_json::from_str::<Value>(&locator).map_err(|_| Error::Format)?}),
        )
    }

    pub fn prepare_extraction(&mut self, item: u64, model: &str) -> Result<ExtractionInput> {
        let (source,title): (String,String) = self.db.query_row("SELECT s.id,s.title FROM collection_item i JOIN source s ON s.id=i.source_id WHERE i.local_id=? AND i.kind='document' AND i.deleted_at IS NULL AND s.sensitivity='personal' AND s.retention='keep'", [crate::vault::sql_id(item)?], |r| Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or(Error::Validation("Release the document content first."))?;
        let segments = {
            let mut stmt = self.db.prepare("SELECT id,ordinal,text FROM source_segment WHERE source_id=? AND ordinal>0 ORDER BY ordinal")?;
            stmt.query_map([&source], |r| {
                Ok(SourceText {
                    segment_id: r.get(0)?,
                    ordinal: r.get(1)?,
                    text: r.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?
        };
        if segments.is_empty() {
            return Err(Error::Validation(
                "Noch kein lesbarer Dokumenttext vorhanden.",
            ));
        }
        if segments.iter().map(|s| s.text.len()).sum::<usize>() > crate::MAX_DOCUMENT_TEXT {
            return Err(Error::Validation(
                "Document text exceeds 8 MiB. Split the file and try again.",
            ));
        }
        let tx = self.db.transaction()?;
        let key = format!("facts-v1:{source}");
        let state: Option<String> = tx
            .query_row("SELECT state FROM job WHERE dedup_key=?", [&key], |r| {
                r.get(0)
            })
            .optional()?;
        if state.is_some_and(|s| {
            !["failed", "waiting_provider", "done", "needs_review"].contains(&s.as_str())
        }) {
            return Err(Error::Validation(
                "This file has been analyzed or is awaiting review.",
            ));
        }
        let run = id();
        tx.execute("INSERT INTO job(id,source_id,kind,dedup_key,state,attempts) VALUES(?,?,'extract_facts',?,'running',1) ON CONFLICT(dedup_key) DO UPDATE SET state='running',attempts=attempts+1,last_error_code=NULL",params![id(),source,key])?;
        tx.execute("INSERT INTO extraction_run VALUES(?,?, 'codex-app-server',?,'document-v3',1,strftime('%Y-%m-%dT%H:%M:%fZ','now'),'running')",params![run,source,model])?;
        tx.commit()?;
        Ok(ExtractionInput {
            run_id: run,
            source_id: source,
            title,
            segments,
        })
    }

    pub fn fail_extraction(&mut self, run: &str) -> Result<()> {
        let tx = self.db.transaction()?;
        tx.execute("UPDATE job SET state='waiting_provider',last_error_code='provider_or_validation_failed' WHERE kind='extract_facts' AND source_id=(SELECT source_id FROM extraction_run WHERE id=? AND status='running')",[run])?;
        tx.execute(
            "UPDATE extraction_run SET status='failed' WHERE id=? AND status='running'",
            [run],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn finish_extraction(
        &mut self,
        input: &ExtractionInput,
        output: ExtractionOutput,
    ) -> Result<usize> {
        self.finish_extraction_with_questions(input, output, Vec::new())
    }

    pub fn finish_extraction_with_questions(
        &mut self,
        input: &ExtractionInput,
        output: ExtractionOutput,
        questions: Vec<crate::RejectedFact>,
    ) -> Result<usize> {
        if output.facts.len() + questions.len() > MAX_EXTRACTION_FACTS {
            return Err(Error::Validation("Too many suggestions in one response."));
        }
        let active: bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM extraction_run e JOIN source s ON s.id=e.source_id WHERE e.id=? AND e.source_id=? AND e.status='running' AND s.sensitivity='personal' AND s.retention='keep')",params![input.run_id,input.source_id],|r|r.get(0))?;
        if !active {
            return Err(Error::Validation("This analysis is no longer active."));
        }
        let mut checked = Vec::new();
        for fact in output.facts {
            let segment: String = self
                .db
                .query_row(
                    "SELECT text FROM source_segment WHERE id=? AND source_id=? AND ordinal>0",
                    params![fact.segment_id, input.source_id],
                    |r| r.get(0),
                )
                .optional()?
                .ok_or(Error::Validation(
                    "Die KI hat eine unbekannte Fundstelle genannt.",
                ))?;
            let subject_present: bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM source_segment WHERE source_id=? AND ordinal>0 AND instr(text,?)>0)",params![input.source_id,fact.subject_quote],|r|r.get(0))?;
            if fact.quote.is_empty()
                || fact.quote.len() > 4000
                || !segment.contains(&fact.quote)
                || !crate::grounding::contains_whole_value(&fact.quote, &fact.value)
                || fact.subject_quote.is_empty()
                || fact.subject_quote.len() > 1000
                || !subject_present
                || fact.context_quote.len() > 1000
                || (!fact.context_quote.is_empty() && !self.db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM source_segment WHERE source_id=? AND ordinal>0 AND instr(text,?)>0)",
                    params![input.source_id,fact.context_quote], |r| r.get::<_,bool>(0))?)
            {
                return Err(Error::Validation(
                    "A suggested detail isn't supported by the original text.",
                ));
            }
            let normalized = normalize_fact(&fact.property, &fact.value)?;
            checked.push((fact, normalized));
        }
        let tx = self.db.transaction()?;
        let mut count = 0;
        crate::review_questions::store_questions(&tx, &input.source_id, &input.run_id, questions)?;
        for (fact, value) in checked {
            let key = crate::review_questions::candidate_key(&fact);
            let answered: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM ai_question WHERE source_id=? AND candidate_key=? AND state IN ('confirmed','dismissed'))", params![input.source_id,key], |r|r.get(0))?;
            if answered {
                continue;
            }
            tx.execute("UPDATE ai_question SET state='resolved' WHERE source_id=? AND candidate_key=? AND state='pending'",params![input.source_id,key])?;
            let property =
                crate::extraction_fields::ensure_extraction_property(&tx, &input.source_id, &fact)?;
            count += tx.execute(
                "INSERT OR IGNORE INTO ai_proposal VALUES(?,?,?,?,?,?,?,'proposed',?)",
                params![
                    id(),
                    input.source_id,
                    property,
                    value,
                    fact.quote,
                    fact.segment_id,
                    fact.subject_quote,
                    input.run_id
                ],
            )?;
        }
        tx.execute(
            "UPDATE extraction_run SET status='succeeded' WHERE id=?",
            [&input.run_id],
        )?;
        let pending: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM ai_proposal WHERE source_id=?1 AND state='proposed') OR EXISTS(SELECT 1 FROM ai_question WHERE source_id=?1 AND state='pending')",
            [&input.source_id],
            |r| r.get(0),
        )?;
        tx.execute("UPDATE job SET state=?,last_error_code=NULL WHERE kind='extract_facts' AND source_id=?",params![if pending {"needs_review"}else{"done"},input.source_id])?;
        crate::review_questions::refresh_review_state(&tx, &input.source_id)?;
        tx.commit()?;
        Ok(count)
    }

    pub fn proposals(&self, item: u64) -> Result<Vec<Proposal>> {
        let mut stmt=self.db.prepare("SELECT p.id,d.label,p.value,p.quote,p.subject_quote,g.locator_json FROM ai_proposal p LEFT JOIN source_segment g ON g.id=p.segment_id JOIN property_definition d ON d.key=p.property_key JOIN collection_item i ON i.source_id=p.source_id WHERE i.local_id=? AND p.state='proposed' ORDER BY p.property_key")?;
        let mut proposals = stmt
            .query_map([crate::vault::sql_id(item)?], |r| {
                Ok(Proposal {
                    id: r.get(0)?,
                    label: r.get(1)?,
                    value: r.get(2)?,
                    quote: r.get(3)?,
                    subject_quote: r.get(4)?,
                    existing_values: Vec::new(),
                    location_label: {
                        let raw: Option<String> = r.get(5)?;
                        let loc: Value = raw
                            .and_then(|v| serde_json::from_str(&v).ok())
                            .unwrap_or(Value::Null);
                        let mut label = loc["page"]
                            .as_u64()
                            .map(|p| format!("Page {p}"))
                            .or_else(|| loc["section"].as_str().map(str::to_string))
                            .unwrap_or_else(|| "Source".into());
                        if loc["method"] == "ocr" {
                            label.push_str(" · OCR");
                        }
                        label
                    },
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for proposal in &mut proposals {
            let mut existing=self.db.prepare("SELECT DISTINCT value_json FROM assertion_state WHERE state='accept' AND subject_id=(SELECT profile_id FROM vault_meta) AND property_key=(SELECT property_key FROM ai_proposal WHERE id=?) LIMIT 20")?;
            let values = existing
                .query_map([&proposal.id], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            proposal.existing_values = values
                .iter()
                .filter_map(|v| serde_json::from_str(v).ok())
                .collect();
        }
        Ok(proposals)
    }

    /// Only the local review UI may call this. The MCP surface is read-only.
    pub fn review_proposals(&mut self, item: u64, accept: bool) -> Result<()> {
        let tx = self.db.transaction()?;
        review_proposals_tx(&tx, item, None, accept)?;
        tx.commit()?;
        Ok(())
    }
}

pub(crate) fn review_proposals_tx(
    tx: &rusqlite::Transaction<'_>,
    item: u64,
    selected: Option<&str>,
    accept: bool,
) -> Result<()> {
    let profile: String = tx.query_row("SELECT profile_id FROM vault_meta", [], |r| r.get(0))?;
    let mut stmt=tx.prepare("SELECT p.id,p.source_id,p.property_key,p.value,p.quote,p.segment_id,p.run_id FROM ai_proposal p JOIN collection_item i ON i.source_id=p.source_id WHERE i.local_id=? AND p.state='proposed' AND (?2 IS NULL OR p.id=?2) AND i.deleted_at IS NULL AND NOT EXISTS(SELECT 1 FROM extraction_run e WHERE e.source_id=p.source_id AND e.status='running')")?;
    let rows = stmt
        .query_map(params![crate::vault::sql_id(item)?, selected], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(stmt);
    if selected.is_some() && rows.len() != 1 {
        return Err(Error::Validation("This item is no longer awaiting review."));
    }
    for (proposal, source, property, value, quote, segment, run) in rows {
        if accept {
            let (label, value_type): (String, String) = tx.query_row(
                "SELECT label,value_type FROM property_definition WHERE key=?",
                [&property],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
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
            tx.execute("INSERT OR IGNORE INTO assertion(id,subject_id,property_key,value_type,value_json,canonical_value,semantic_key,time_kind,recorded_at,origin,extraction_run_id) VALUES(?,?,?,?,?,?,?,'unknown',strftime('%Y-%m-%dT%H:%M:%fZ','now'),'extraction',?)",params![assertion,profile,property,value_type,raw,raw,semantic,run])?;
            decision(tx, &assertion, "accept")?;
            let source_locator: Option<String> = tx
                .query_row(
                    "SELECT locator_json FROM source_segment WHERE id=? AND source_id=?",
                    params![segment, source],
                    |r| r.get(0),
                )
                .optional()?;
            let locator = json!({"segment_id":segment,"quote":quote,"source_locator":source_locator.and_then(|v|serde_json::from_str::<Value>(&v).ok())});
            tx.execute(
                "INSERT OR IGNORE INTO assertion_evidence VALUES(?,?,?,?)",
                params![
                    assertion,
                    source,
                    hash(locator.to_string().as_bytes()),
                    locator.to_string()
                ],
            )?;
            let stable = id();
            tx.execute("INSERT INTO collection_item(stable_id,title,kind,source_id,property_key,current_assertion_id) VALUES(?,?,'note',?,?,?)",params![stable,label,source,property,assertion])?;
            journal(
                tx,
                &stable,
                1,
                "extraction_accepted",
                json!({"assertion_id":assertion,"source_id":source}),
            )?;
        }
        tx.execute(
            "UPDATE ai_proposal SET state=? WHERE id=?",
            params![if accept { "accepted" } else { "rejected" }, proposal],
        )?;
    }
    let source: String = tx.query_row(
        "SELECT source_id FROM collection_item WHERE local_id=?",
        [crate::vault::sql_id(item)?],
        |r| r.get(0),
    )?;
    crate::review_questions::refresh_review_state(tx, &source)?;
    Ok(())
}
