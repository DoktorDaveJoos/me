//! The read store: profile values go through Resolve; every other grounded value
//! is a document fact of its source; values nobody interpreted stay visible.
use crate::{
    DocumentGraph, Error, Result, Uncovered, Vault,
    graph::{GRAPH_POLICY, policy_decision, resolve_document},
    vault::id,
};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

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

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ReadOutcome {
    pub values: usize,
    pub in_profile: usize,
    pub checks: usize,
    pub uninterpreted: usize,
}

fn locator(source: &str, segment: &str, start: usize, end: usize) -> String {
    json!({"kind":"candidate","source_id":source,"segment_id":segment,"start":start,"end":end})
        .to_string()
}

impl Vault {
    /// Replaces this source's read: document facts, automatic profile values and
    /// the summary. User decisions are never overridden.
    pub fn apply_read(&mut self, source: &str, read: &DocumentRead) -> Result<ReadOutcome> {
        let tx = self.db.transaction()?;
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
        // Automatic values this source supported so far; a person's decision wins.
        let previous: BTreeSet<String> = tx
            .prepare("SELECT DISTINCT a.id FROM assertion_state a JOIN assertion_evidence e ON e.assertion_id=a.id WHERE e.source_id=? AND a.origin='extraction' AND a.state='accept' AND (SELECT actor FROM decision d WHERE d.assertion_id=a.id ORDER BY d.local_seq DESC LIMIT 1)='policy'")?
            .query_map([source], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<_, _>>()?;
        let graph = match &read.graph {
            Some(g) => Some(resolve_document(&tx, source, g)?),
            None => None,
        };
        let recorded = graph
            .as_ref()
            .map(|g| g.recorded.clone())
            .unwrap_or_default();
        for stale in previous.difference(&recorded) {
            policy_decision(&tx, stale, "retract", "not_found_on_reread")?;
            tx.execute(
                "UPDATE collection_item SET deleted_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE current_assertion_id=? AND kind='note' AND deleted_at IS NULL",
                [stale],
            )?;
        }
        tx.execute("DELETE FROM document_fact WHERE source_id=?", [source])?;
        let slots = graph
            .as_ref()
            .map(|g| g.slot_assertions.clone())
            .unwrap_or_default();
        let mut in_profile = 0;
        for (ordinal, f) in read.facts.iter().enumerate() {
            let assertion = f.slot.as_ref().and_then(|s| slots.get(s)).cloned();
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
        let checks = graph.as_ref().map_or(0, |g| g.checks);
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
        tx.commit()?;
        Ok(outcome)
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
