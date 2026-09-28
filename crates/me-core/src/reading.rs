//! The read store: profile values go through Resolve; every other grounded value
//! is a document fact of its source; values nobody interpreted stay visible.
use crate::{
    DocumentGraph, Error, Result, Uncovered, Vault,
    graph::{GRAPH_POLICY, resolve_read, source_checks},
    vault::id,
};
use rusqlite::{Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactState {
    Verified,
    Uncertain,
    Unverified,
    Uninterpreted,
}
impl FactState {
    pub fn key(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Uncertain => "uncertain",
            Self::Unverified => "unverified",
            Self::Uninterpreted => "uninterpreted",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReadFact {
    pub label: String,
    /// Raw printed value.
    pub value: String,
    pub segment_id: String,
    pub start: usize,
    pub end: usize,
    pub context: String,
    /// `self`, `household`, `party`, `organization`, `unclear` or `unknown`.
    pub owner: String,
    pub owner_entity: Option<String>,
    pub owner_name: Option<String>,
    /// `document`, `cumulative`, `other` or `none` for amounts.
    pub period: Option<String>,
    /// Registry slot this fact fills in the profile, if any.
    pub slot: Option<String>,
    pub state: FactState,
    pub confidence: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DocumentRead {
    pub run_id: String,
    pub graph: Option<DocumentGraph>,
    pub facts: Vec<ReadFact>,
    pub uninterpreted: Vec<Uncovered>,
    /// Grounding rejections by code (counts only).
    pub rejected: BTreeMap<String, usize>,
}

/// One value read from a document, as the document detail shows it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReadRow {
    pub label: String,
    /// Raw printed value.
    pub value: String,
    /// The period, account or section the document states for this value.
    pub period: Option<String>,
    /// Page or section of the source, e.g. "Page 1".
    pub location: String,
    /// The reading could not be fully verified.
    pub uncertain: bool,
    /// The profile value this fact is in now, if it is accepted.
    pub assertion: Option<String>,
    /// The linked profile value is waiting in Quick checks.
    pub check: bool,
}

/// What was read from one document: values now in the profile, the other details
/// it states, and printed values nobody interpreted.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct DocumentReadView {
    pub in_profile: Vec<ReadRow>,
    pub other: Vec<ReadRow>,
    pub uninterpreted: Vec<ReadRow>,
}

impl DocumentReadView {
    pub fn is_empty(&self) -> bool {
        self.in_profile.is_empty() && self.other.is_empty() && self.uninterpreted.is_empty()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ReadOutcome {
    pub values: usize,
    pub in_profile: usize,
    pub checks: usize,
    pub uninterpreted: usize,
}

/// Owners and amount periods the read store accepts (the `document_fact` checks).
const OWNERS: &[&str] = &[
    "self",
    "household",
    "party",
    "organization",
    "unclear",
    "unknown",
];
const PERIODS: &[&str] = &["document", "cumulative", "other", "none"];

fn validate(facts: &[ReadFact]) -> Result<()> {
    for fact in facts {
        if !OWNERS.contains(&fact.owner.as_str()) {
            return Err(Error::Validation("A read value has an unknown owner."));
        }
        if fact
            .period
            .as_deref()
            .is_some_and(|p| !PERIODS.contains(&p))
        {
            return Err(Error::Validation("A read value has an unknown period."));
        }
    }
    Ok(())
}

fn locator(source: &str, segment: &str, start: usize, end: usize) -> String {
    json!({"kind":"candidate","source_id":source,"segment_id":segment,"start":start,"end":end})
        .to_string()
}

/// Only personal documents that are kept are read.
fn check_personal(tx: &Transaction<'_>, source: &str) -> Result<()> {
    let personal: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM source WHERE id=? AND sensitivity='personal' AND retention='keep')",
        [source],
        |r| r.get(0),
    )?;
    if !personal {
        return Err(Error::Validation(
            "This document is not available for analysis.",
        ));
    }
    Ok(())
}

/// Replaces this source's document facts and summary. `links` maps slots to the
/// profile assertions that carry them now; `checks` counts the source's standing
/// quick checks.
fn store_document_facts(
    tx: &Transaction<'_>,
    source: &str,
    read: &DocumentRead,
    links: &BTreeMap<String, String>,
    checks: usize,
) -> Result<ReadOutcome> {
    tx.execute("DELETE FROM document_fact WHERE source_id=?", [source])?;
    let mut in_profile = 0;
    for (ordinal, f) in read.facts.iter().enumerate() {
        let assertion = f.slot.as_ref().and_then(|s| links.get(s)).cloned();
        in_profile += usize::from(assertion.is_some());
        tx.execute(
            "INSERT INTO document_fact VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
            params![
                id(),
                source,
                read.run_id,
                ordinal as i64,
                f.label,
                f.value,
                f.context,
                locator(source, &f.segment_id, f.start, f.end),
                f.owner,
                f.owner_entity,
                f.owner_name,
                f.period,
                f.slot,
                assertion,
                f.state.key(),
                f.confidence.map(|c| c.clamp(0., 1.))
            ],
        )?;
    }
    let base = read.facts.len();
    for (i, u) in read.uninterpreted.iter().enumerate() {
        tx.execute(
            "INSERT INTO document_fact VALUES(?,?,?,?,?,?,'',?,'unknown',NULL,NULL,NULL,NULL,NULL,'uninterpreted',NULL,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
            params![
                id(),
                source,
                read.run_id,
                (base + i) as i64,
                u.label.clone().unwrap_or_default(),
                u.text,
                locator(source, &u.segment_id, u.start, u.end)
            ],
        )?;
    }
    let outcome = ReadOutcome {
        values: read.facts.len(),
        in_profile,
        checks,
        uninterpreted: read.uninterpreted.len(),
    };
    tx.execute(
        "INSERT INTO read_summary VALUES(?,?,?,?,?,?,?,?,strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(source_id) DO UPDATE SET run_id=excluded.run_id,policy=excluded.policy,values_read=excluded.values_read,in_profile=excluded.in_profile,checks=excluded.checks,uninterpreted=excluded.uninterpreted,rejected_json=excluded.rejected_json,recorded_at=excluded.recorded_at",
        params![
            source,
            read.run_id,
            GRAPH_POLICY,
            outcome.values as i64,
            outcome.in_profile as i64,
            outcome.checks as i64,
            outcome.uninterpreted as i64,
            serde_json::to_string(&read.rejected).map_err(|_| Error::Format)?
        ],
    )?;
    Ok(outcome)
}

impl Vault {
    /// Replaces this source's verified read: document facts, automatic profile
    /// values and the summary. A verified read without profile values (a type that
    /// fills none) withdraws this source's earlier automatic values like any
    /// re-read that no longer states them. A read whose verification failed goes
    /// through [`Vault::store_unverified_read`] instead. User decisions are never
    /// overridden.
    pub fn apply_read(&mut self, source: &str, read: &DocumentRead) -> Result<ReadOutcome> {
        validate(&read.facts)?;
        let tx = self.db.transaction()?;
        check_personal(&tx, source)?;
        // Profile values replace this source's earlier automatic values; a person's
        // decision wins. Checks are counted after every retraction of this read.
        let graph = resolve_read(&tx, source, read.graph.as_ref())?;
        let outcome =
            store_document_facts(&tx, source, read, &graph.slot_assertions, graph.checks)?;
        tx.commit()?;
        Ok(outcome)
    }

    /// Stores a read whose assumptions could not be verified: only this source's
    /// document facts (all unverified, plus the values nobody interpreted) and its
    /// summary are replaced. Profile values, their evidence and every decision stay
    /// exactly as they are; nothing is resolved, retracted or withdrawn.
    pub fn store_unverified_read(
        &mut self,
        source: &str,
        read: &DocumentRead,
    ) -> Result<ReadOutcome> {
        validate(&read.facts)?;
        if read.graph.is_some()
            || read
                .facts
                .iter()
                .any(|f| f.state != FactState::Unverified || f.slot.is_some())
        {
            return Err(Error::Validation(
                "An unverified read stores only unverified document facts.",
            ));
        }
        let tx = self.db.transaction()?;
        check_personal(&tx, source)?;
        let checks = source_checks(&tx, source)?;
        let outcome = store_document_facts(&tx, source, read, &BTreeMap::new(), checks)?;
        tx.commit()?;
        Ok(outcome)
    }

    /// What was read from this collection item's document, grouped for its detail
    /// view. A fact counts as in the profile only while its linked value is
    /// accepted; a value the person rejected leaves its fact as another detail.
    pub fn document_read(&self, item: u64) -> Result<DocumentReadView> {
        let mut stmt = self.db.prepare("SELECT f.label,f.value,f.context_quote,g.locator_json,f.state,a.id,coalesce(r.check_reason IS NOT NULL AND (SELECT d.actor FROM decision d WHERE d.assertion_id=a.id ORDER BY d.local_seq DESC LIMIT 1)='policy',0) FROM document_fact f JOIN collection_item i ON i.source_id=f.source_id JOIN source s ON s.id=f.source_id LEFT JOIN source_segment g ON g.id=json_extract(f.locator_json,'$.segment_id') AND g.source_id=f.source_id LEFT JOIN assertion_state a ON a.id=f.assertion_id AND a.state='accept' AND a.subject_id IN (SELECT id FROM entity WHERE deleted_at IS NULL) LEFT JOIN assertion_review r ON r.assertion_id=a.id WHERE i.local_id=? AND s.sensitivity='personal' AND s.retention='keep' ORDER BY f.ordinal")?;
        let rows = stmt.query_map([crate::vault::sql_id(item)?], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, bool>(6)?,
            ))
        })?;
        let mut view = DocumentReadView::default();
        for row in rows {
            let (label, value, context, locator, state, assertion, check) = row?;
            let entry = ReadRow {
                label,
                value,
                period: (!context.trim().is_empty()).then_some(context),
                location: crate::knowledge::location_label(locator.as_deref()),
                uncertain: state == FactState::Uncertain.key()
                    || state == FactState::Unverified.key(),
                assertion,
                check,
            };
            if state == FactState::Uninterpreted.key() {
                view.uninterpreted.push(entry);
            } else if entry.assertion.is_some() {
                view.in_profile.push(entry);
            } else {
                view.other.push(entry);
            }
        }
        Ok(view)
    }

    /// Counts-only diagnosis of every read document, safe to share: no titles,
    /// labels, values or names. Documents are numbered in read order.
    pub fn read_self_check(&self) -> Result<String> {
        let mut stmt = self.db.prepare("SELECT coalesce(p.family,'unclassified'),coalesce(p.doc_type,'-'),r.values_read,r.in_profile,r.checks,r.uninterpreted,r.rejected_json,r.policy FROM read_summary r LEFT JOIN document_profile p ON p.source_id=r.source_id ORDER BY r.recorded_at")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
            ))
        })?;
        let mut out = String::from("ME. extraction self-check (counts only)\n");
        for (n, row) in rows.enumerate() {
            let (family, doc_type, values, profile, checks, open, rejected, policy) = row?;
            let rejected: BTreeMap<String, usize> =
                serde_json::from_str(&rejected).unwrap_or_default();
            let codes = rejected
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(",");
            out.push_str(&format!(
                "#{} {family}/{doc_type} · read {values} · profile {profile} · checks {checks} · not interpreted {open} · rejected {} · {policy}\n",
                n + 1,
                if codes.is_empty() { "none".into() } else { codes }
            ));
        }
        Ok(out)
    }

    /// Marks a read's extraction run and job finished.
    pub fn complete_read_run(&mut self, run_id: &str) -> Result<()> {
        let tx = self.db.transaction()?;
        tx.execute(
            "UPDATE extraction_run SET status='succeeded' WHERE id=? AND status='running'",
            [run_id],
        )?;
        tx.execute(
            "UPDATE job SET state='done',last_error_code=NULL WHERE kind='extract_facts' AND source_id=(SELECT source_id FROM extraction_run WHERE id=?)",
            [run_id],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "reading_tests.rs"]
mod tests;
