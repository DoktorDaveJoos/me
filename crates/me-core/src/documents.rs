//! Restartable local document extraction. Platform parsers run outside the vault lock.
use crate::{
    Error, Result, Vault,
    vault::{hash, id, sql_id},
};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::json;
use zeroize::Zeroizing;

pub const MAX_DOCUMENT_TEXT: usize = 8 * 1024 * 1024;
pub const MAX_DOCUMENT_PAGES: u32 = 500;
pub const MAX_DOCUMENT_PARTS: usize = MAX_DOCUMENT_PAGES as usize * 2;

/// The source and attempt are opaque so a stale worker cannot complete a newer job.
pub struct DocumentInput {
    source: String,
    attempt: i64,
    pub extension: String,
    pub bytes: Zeroizing<Vec<u8>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentPart {
    pub text: String,
    pub page: Option<u32>,
    pub section: Option<String>,
    pub method: String,
}

impl Vault {
    /// Explicit content release. The caller must finish or fail the returned job.
    /// None means the immutable original has already been indexed successfully.
    pub fn begin_document_processing(&mut self, item: u64) -> Result<Option<DocumentInput>> {
        let (source, extension, sensitivity, object): (String,String,String,String) = self.db.query_row(
            "SELECT s.id,i.extension,s.sensitivity,s.object_id FROM collection_item i JOIN source s ON s.id=i.source_id WHERE i.local_id=? AND i.kind='document' AND i.deleted_at IS NULL AND s.retention='keep'",
            [sql_id(item)?], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
        if sensitivity == "credential" {
            return Err(Error::Validation("Credentials cannot be processed."));
        }
        if !crate::processable_document(&extension) {
            return Err(Error::Validation(
                "File saved. Import a PDF, DOCX, ODT, RTF, or text version for analysis.",
            ));
        }
        let state: String = self.db.query_row(
            "SELECT state FROM job WHERE source_id=? AND kind='index_text'",
            [&source],
            |r| r.get(0),
        )?;
        if sensitivity == "personal" && state == "done" {
            return Ok(None);
        }
        if state == "running" {
            return Err(Error::Validation("This file is already being processed."));
        }
        let bytes = self.read_object(&object)?;
        let tx = self.db.transaction()?;
        tx.execute(
            "UPDATE source SET sensitivity='personal' WHERE id=?",
            [&source],
        )?;
        let attempt = tx.query_row("UPDATE job SET state='running',attempts=attempts+1,last_error_code=NULL WHERE source_id=? AND kind='index_text' RETURNING attempts", [&source], |r| r.get(0))?;
        tx.commit()?;
        Ok(Some(DocumentInput {
            source,
            attempt,
            extension,
            bytes,
        }))
    }

    pub fn fail_document_processing(&mut self, input: &DocumentInput) -> Result<()> {
        self.db.execute("UPDATE job SET state='failed',last_error_code='document_extraction_failed' WHERE source_id=? AND kind='index_text' AND state='running' AND attempts=?", params![input.source,input.attempt])?;
        Ok(())
    }

    pub fn finish_document_processing(
        &mut self,
        input: &DocumentInput,
        parts: &[DocumentPart],
    ) -> Result<()> {
        if parts.is_empty()
            || parts.len() > MAX_DOCUMENT_PARTS
            || parts.iter().map(|p| p.text.len()).sum::<usize>() > MAX_DOCUMENT_TEXT
        {
            return Err(Error::Validation(
                "Document text is missing or exceeds 8 MiB.",
            ));
        }
        if parts.iter().all(|p| p.text.trim().is_empty()) {
            return Err(Error::Validation(
                "No readable text found. Check the original's resolution and orientation.",
            ));
        }
        for part in parts {
            if part.text.contains('\0')
                || !["text", "pdf_text", "ocr", "office"].contains(&part.method.as_str())
                || part.page.is_some_and(|p| p == 0 || p > MAX_DOCUMENT_PAGES)
                || part.section.as_ref().is_some_and(|s| s.len() > 200)
            {
                return Err(Error::Validation(
                    "Document processing returned an invalid result.",
                ));
            }
        }
        let tx = self.db.transaction()?;
        let active: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM job j JOIN source s ON s.id=j.source_id WHERE j.source_id=? AND j.kind='index_text' AND j.state='running' AND j.attempts=? AND s.sensitivity='personal' AND s.retention='keep')",params![input.source,input.attempt],|r|r.get(0))?;
        if !active {
            return Err(Error::Validation("This document job is no longer active."));
        }
        // No reindexing while facts reference existing segments. Normally unreachable:
        // successful immutable sources return None above.
        let has_facts: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM extraction_run WHERE source_id=?)",
            [&input.source],
            |r| r.get(0),
        )?;
        if has_facts {
            return Err(Error::Validation("This file has already been analyzed."));
        }
        let revision: i64 = tx.query_row("UPDATE source SET extraction_revision=coalesce(extraction_revision,0)+1 WHERE id=? RETURNING extraction_revision",[&input.source],|r|r.get::<_,Option<i64>>(0))?.unwrap_or(1);
        tx.execute(
            "DELETE FROM source_segment WHERE source_id=? AND ordinal>0",
            [&input.source],
        )?;
        let mut ordinal = 1;
        for (part_index, part) in parts.iter().enumerate() {
            let mut start = 0;
            while start < part.text.len() {
                let mut end = (start + 1600).min(part.text.len());
                while !part.text.is_char_boundary(end) {
                    end -= 1;
                }
                if end < part.text.len()
                    && let Some(newline) = part.text[start..end].rfind('\n')
                    && newline > 800
                {
                    end = start + newline + 1;
                }
                let text = &part.text[start..end];
                let locator = json!({"kind":"document_text","method":part.method,"page":part.page,"section":part.section,"part":part_index+1,"start_byte":start,"end_byte":end});
                tx.execute("INSERT INTO source_segment(id,source_id,extraction_revision,ordinal,locator_json,text,content_hash) VALUES(?,?,?,?,?,?,?)",params![id(),input.source,revision,ordinal,locator.to_string(),text,hash(text.as_bytes())])?;
                ordinal += 1;
                start = end;
            }
        }
        tx.execute("UPDATE job SET state='done',last_error_code=NULL WHERE source_id=? AND kind='index_text'",[&input.source])?;
        tx.execute("UPDATE inbox_item SET state='done' WHERE id IN (SELECT inbox_id FROM inbox_source WHERE source_id=?)",[&input.source])?;
        tx.commit()?;
        Ok(())
    }
}
