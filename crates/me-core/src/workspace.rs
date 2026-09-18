//! Data for the chat, review queue and virtual document tree.
use crate::{Content, Error, Result, Vault, vault::sql_id};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct ReviewEntry {
    pub id: String,
    pub item: u64,
    pub title: String,
    pub label: String,
    pub value: String,
    pub reason: String,
    pub existing_values: Vec<String>,
    pub question: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FolderAssignment {
    pub item: u64,
    pub path: Vec<String>,
}

impl Vault {
    pub fn pending_reviews(&self) -> Result<Vec<ReviewEntry>> {
        let mut entries = Vec::new();
        for item in self.collection("", false)?.items {
            if !matches!(item.content, Content::Document { .. }) {
                continue;
            }
            for p in self.proposals(item.id)? {
                entries.push(ReviewEntry {
                    id: p.id,
                    item: item.id,
                    title: item.title.clone(),
                    label: p.label,
                    value: p.value,
                    reason: "Confirm this belongs to you.".into(),
                    existing_values: p.existing_values,
                    question: false,
                });
            }
            for q in self.review_questions(item.id)? {
                entries.push(ReviewEntry {
                    id: q.id,
                    item: item.id,
                    title: item.title.clone(),
                    label: q.label,
                    value: q.value,
                    reason: q.reason,
                    existing_values: q.existing_values,
                    question: true,
                });
            }
        }
        Ok(entries)
    }

    /// A batch is one explicit user decision. Re-read values inside the transaction;
    /// a stale selection or invalid answer rolls back the entire batch.
    pub fn review_selected(&mut self, ids: &[String], accept: bool) -> Result<()> {
        if ids.is_empty()
            || ids.len() > 1024
            || ids.iter().collect::<BTreeSet<_>>().len() != ids.len()
        {
            return Err(Error::Validation(
                "Select between 1 and 1,024 different items.",
            ));
        }
        let tx = self.db.transaction()?;
        for key in ids {
            let question: Option<(i64, String)> = tx.query_row(
                "SELECT i.local_id,q.value FROM ai_question q JOIN collection_item i ON i.source_id=q.source_id WHERE q.id=? AND q.state='pending' AND i.kind='document' AND i.deleted_at IS NULL", [key], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
            if let Some((item, value)) = question {
                crate::review_questions::answer_question_tx(
                    &tx,
                    item as u64,
                    key,
                    accept.then_some(value.as_str()),
                )?;
            } else {
                let item: Option<i64> = tx.query_row("SELECT i.local_id FROM ai_proposal p JOIN collection_item i ON i.source_id=p.source_id JOIN source s ON s.id=p.source_id WHERE p.id=? AND p.state='proposed' AND i.kind='document' AND i.deleted_at IS NULL AND s.sensitivity='personal' AND s.retention='keep'", [key], |r| r.get(0)).optional()?;
                crate::knowledge::review_proposals_tx(
                    &tx,
                    item.ok_or(Error::Validation("This item is no longer awaiting review."))?
                        as u64,
                    Some(key),
                    accept,
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn document_folders(&self) -> Result<BTreeMap<u64, String>> {
        let mut stmt = self.db.prepare("SELECT f.item_id,f.path FROM document_folder f JOIN collection_item i ON i.local_id=f.item_id WHERE i.deleted_at IS NULL")?;
        Ok(stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)? as u64, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?)
    }

    /// Only already released, locally indexed files may be sent for organization.
    pub fn organization_input(&self) -> Result<Value> {
        let mut stmt = self.db.prepare("SELECT i.local_id,i.title,(SELECT substr(group_concat(g.text, char(10)),1,2400) FROM source_segment g WHERE g.source_id=i.source_id AND g.ordinal>0) FROM collection_item i JOIN source s ON s.id=i.source_id WHERE i.kind='document' AND i.deleted_at IS NULL AND s.sensitivity='personal' AND s.retention='keep' AND EXISTS(SELECT 1 FROM document_evaluation e WHERE e.source_id=i.source_id AND e.state='done') AND NOT EXISTS(SELECT 1 FROM document_folder f WHERE f.item_id=i.local_id) AND EXISTS(SELECT 1 FROM source_segment g WHERE g.source_id=i.source_id AND g.ordinal>0) ORDER BY i.local_id LIMIT 20")?;
        let docs = stmt.query_map([], |r| Ok(json!({"item":r.get::<_, i64>(0)?,"title":r.get::<_, String>(1)?,"text":r.get::<_, Option<String>>(2)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let paths: BTreeSet<_> = self.document_folders()?.into_values().collect();
        Ok(json!({"documents":docs,"existing_folders":paths}))
    }

    pub fn save_document_folders(
        &mut self,
        assignments: &[FolderAssignment],
        allowed: &[u64],
    ) -> Result<()> {
        let tx = self.db.transaction()?;
        let mut seen = BTreeSet::new();
        for assignment in assignments {
            if !allowed.contains(&assignment.item)
                || !seen.insert(assignment.item)
                || assignment.path.is_empty()
                || assignment.path.len() > 3
                || assignment.path.iter().any(|s| {
                    s.trim().is_empty()
                        || s.len() > 64
                        || s == "."
                        || s == ".."
                        || s.chars().any(|c| c.is_control() || c == '/' || c == '\\')
                })
            {
                return Err(Error::Validation(
                    "The suggested folder is invalid. Try organizing again.",
                ));
            }
            let path = assignment
                .path
                .iter()
                .map(|s| s.trim())
                .collect::<Vec<_>>()
                .join("/");
            let count = tx.execute("INSERT INTO document_folder(item_id,path) SELECT i.local_id,? FROM collection_item i JOIN source s ON s.id=i.source_id WHERE i.local_id=? AND i.kind='document' AND i.deleted_at IS NULL AND s.sensitivity='personal' AND s.retention='keep' ON CONFLICT(item_id) DO UPDATE SET path=excluded.path",params![path,sql_id(assignment.item)?])?;
            if count != 1 {
                return Err(Error::Validation("The document is no longer available."));
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Bounded source context; credentials and unreleased documents are excluded.
    pub fn chat_context(
        &self,
        queries: &[String],
        fields: &[String],
        attachments: &[u64],
    ) -> Result<Value> {
        let scope = self.shareable_scope()?;
        let facts = if fields.is_empty() {
            json!({"facts":[]})
        } else {
            self.facts_get(&scope, fields)?
        };
        let mut evidence = BTreeMap::<String, Value>::new();
        for query in queries.iter().take(5) {
            if query.trim().is_empty() {
                continue;
            }
            let hits = self.documents_search(&scope, query, 6)?;
            for hit in hits["hits"].as_array().into_iter().flatten() {
                if let Some(segment) = hit["segment_id"].as_str() {
                    evidence
                        .entry(segment.into())
                        .or_insert_with(|| hit.clone());
                }
            }
        }
        for item in attachments.iter().take(8) {
            let source: Option<String> = self.db.query_row("SELECT s.id FROM collection_item i JOIN source s ON s.id=i.source_id WHERE i.local_id=? AND i.deleted_at IS NULL AND s.sensitivity='personal' AND s.retention='keep'",[sql_id(*item)?],|r|r.get(0)).optional()?;
            if let Some(source) = source {
                let mut stmt = self.db.prepare("SELECT id FROM source_segment WHERE source_id=? AND ordinal>0 ORDER BY ordinal LIMIT 12")?;
                for row in stmt.query_map([source], |r| r.get::<_, String>(0))? {
                    let id = row?;
                    let mut e = self.evidence_read(&scope, &id)?;
                    if let Some(text) = e["text"].as_str() {
                        e["text"] = json!(text.chars().take(4500).collect::<String>());
                    }
                    evidence.insert(id, e);
                }
            }
        }
        let source_ids = evidence
            .values()
            .filter_map(|e| e["source_id"].as_str())
            .collect::<BTreeSet<_>>();
        let mut stmt=self.db.prepare("SELECT DISTINCT a.property_key,d.label,a.value_json,s.id,s.title FROM assertion_state a JOIN property_definition d ON d.key=a.property_key JOIN assertion_evidence e ON e.assertion_id=a.id JOIN source s ON s.id=e.source_id WHERE a.state='accept' AND a.subject_id=(SELECT profile_id FROM vault_meta) AND s.sensitivity='personal' AND s.retention='keep' AND a.property_key IN (SELECT aa.property_key FROM assertion_state aa JOIN assertion_evidence ee ON ee.assertion_id=aa.id WHERE aa.state='accept' AND ee.source_id IN (SELECT value FROM json_each(?))) LIMIT 60")?;
        let confirmed=stmt.query_map([json!(source_ids).to_string()],|r| Ok(json!({"property":r.get::<_,String>(0)?,"label":r.get::<_,String>(1)?,"value":serde_json::from_str::<Value>(&r.get::<_,String>(2)?).unwrap_or(Value::Null),"source_id":r.get::<_,String>(3)?,"source_title":r.get::<_,String>(4)?,"state":"accepted"})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        Ok(
            json!({"facts":facts["facts"],"confirmed_details":confirmed,"evidence":evidence.into_values().take(36).collect::<Vec<_>>(),"coverage":"Relevant excerpts only. Source ownership is unconfirmed unless a fact is accepted. Do not infer missing values or absence from the entire vault."}),
        )
    }
}

impl Vault {
    pub fn chat_sources(&self, sources: &[String]) -> Result<Vec<(u64, String)>> {
        let mut result = Vec::new();
        for source in sources.iter().take(8) {
            let row:Option<(i64,String)>=self.db.query_row("SELECT local_id,title FROM collection_item WHERE source_id=? AND deleted_at IS NULL ORDER BY kind='document' DESC LIMIT 1",[source],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
            if let Some((id, title)) = row {
                result.push((id as u64, title));
            }
        }
        Ok(result)
    }
}
