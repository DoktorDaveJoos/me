//! Envelope intake: every source (window drop, folder dump, later a scanner or mail
//! forwarding) produces envelopes of original files. Sources know nothing about
//! stages or AI. Identical content is stored once; a repeat links the existing source.
use crate::{
    DocumentClass, Error, Result, Vault, crypto,
    vault::{MAX_FILE, hash, id, sql_id},
};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use serde_json::json;
use std::path::{Path, PathBuf};

/// Files per folder plan; a larger dump is imported in several runs.
pub const MAX_PLANNED_FILES: usize = 20_000;
/// Default OpenAI gap-fill allowance of one dump batch.
pub const DEFAULT_BATCH_OPENAI_ALLOWANCE: i64 = 200;
const MAX_DEPTH: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Drop,
    Folder,
    Scanner,
    Mail,
}
impl SourceKind {
    fn key(self) -> &'static str {
        match self {
            Self::Drop => "drop",
            Self::Folder => "folder",
            Self::Scanner => "scanner",
            Self::Mail => "mail",
        }
    }
    /// Queue priority: interactive drops outrank a bulk dump or delivery stream.
    fn priority(self) -> i64 {
        match self {
            Self::Drop => 2,
            Self::Folder | Self::Scanner | Self::Mail => 1,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct FolderPlan {
    /// Supported files in walk order.
    pub files: Vec<PathBuf>,
    pub bytes: u64,
    /// Readable files in a format ME. cannot store or read.
    pub unsupported: usize,
    /// Empty, oversized or unreadable files.
    pub skipped: usize,
    /// Hidden and system files (e.g. `.DS_Store`), not counted as documents.
    pub hidden: usize,
    /// The plan stopped at `MAX_PLANNED_FILES`.
    pub truncated: bool,
}

fn hidden(name: &str) -> bool {
    name.starts_with('.')
        || ["thumbs.db", "desktop.ini", "icon\r"].contains(&name.to_ascii_lowercase().as_str())
        || name.starts_with("~$")
}

/// Walks folders (and accepts plain files) without following symbolic links.
/// Metadata only: nothing is read, stored or sent anywhere.
pub fn plan_folder_import(roots: &[PathBuf]) -> FolderPlan {
    let mut plan = FolderPlan::default();
    let mut stack: Vec<(PathBuf, usize)> = roots.iter().rev().map(|p| (p.clone(), 0)).collect();
    while let Some((path, depth)) = stack.pop() {
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            plan.skipped += 1;
            continue;
        };
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if depth > 0 && hidden(name) {
            plan.hidden += 1;
            continue;
        }
        if meta.file_type().is_symlink() {
            plan.skipped += 1;
        } else if meta.is_dir() {
            if depth >= MAX_DEPTH {
                plan.skipped += 1;
                continue;
            }
            let Ok(entries) = std::fs::read_dir(&path) else {
                plan.skipped += 1;
                continue;
            };
            let mut children: Vec<PathBuf> =
                entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
            children.sort();
            stack.extend(children.into_iter().rev().map(|c| (c, depth + 1)));
        } else if meta.is_file() {
            if meta.len() == 0 || meta.len() > MAX_FILE {
                plan.skipped += 1;
            } else if !crate::supported_document(&path) {
                plan.unsupported += 1;
            } else if plan.files.len() >= MAX_PLANNED_FILES {
                plan.truncated = true;
                break;
            } else {
                plan.bytes += meta.len();
                plan.files.push(path);
            }
        } else {
            plan.skipped += 1;
        }
    }
    plan
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct IntakeOutcome {
    pub item: u64,
    /// The same bytes were already in the vault; nothing new was stored.
    pub duplicate: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct BatchProgress {
    pub id: String,
    pub label: String,
    pub files: usize,
    pub duplicates: usize,
    /// Files whose automatic analysis finished (done or failed).
    pub finished: usize,
    pub failed: usize,
    pub openai_calls: i64,
    pub openai_allowance: i64,
    pub typesafe_requests: i64,
}

impl Vault {
    pub fn begin_import_batch(&mut self, label: &str) -> Result<String> {
        crate::domain::bounded(label, 200)?;
        let batch = id();
        self.db.execute("INSERT INTO import_batch(id,label,created_at,openai_allowance) VALUES(?,?,strftime('%Y-%m-%dT%H:%M:%fZ','now'),?)",params![batch,label,DEFAULT_BATCH_OPENAI_ALLOWANCE])?;
        Ok(batch)
    }
    /// Stores one original as its own envelope. Identical content already in the
    /// vault is linked instead of stored twice.
    pub fn import_envelope_file(
        &mut self,
        path: &Path,
        kind: SourceKind,
        batch: Option<&str>,
    ) -> Result<IntakeOutcome> {
        if !crate::supported_document(path) {
            return Err(Error::Validation(
                "Supported files: PDFs, images, Office files, and text up to 64 MiB.",
            ));
        }
        let title = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or(Error::Validation("Invalid filename."))?
            .to_owned();
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let bytes = crypto::read_bounded(path, MAX_FILE)?;
        let fingerprint = hash(&bytes);
        let existing: Option<(i64, String)> = self.db.query_row("SELECT i.local_id,s.id FROM source s JOIN collection_item i ON i.source_id=s.id WHERE s.content_fingerprint=? AND s.kind='file' AND s.retention='keep' AND i.kind='document' AND i.deleted_at IS NULL ORDER BY i.local_id LIMIT 1",[&fingerprint],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let (item, source, duplicate) = match existing {
            Some((item, source)) => (item as u64, source, true),
            None => {
                let item = self.import_document_bytes(
                    &bytes,
                    &id(),
                    DocumentClass::Unclassified,
                    &title,
                    &extension,
                )?;
                let source: String = self.db.query_row(
                    "SELECT source_id FROM collection_item WHERE local_id=?",
                    [sql_id(item)?],
                    |r| r.get(0),
                )?;
                self.db.execute(
                    "UPDATE document_evaluation SET priority=? WHERE source_id=?",
                    params![kind.priority(), source],
                )?;
                (item, source, false)
            }
        };
        let envelope = id();
        let origin = json!({"file_name":title});
        let tx = self.db.transaction()?;
        tx.execute(
            "INSERT INTO intake_envelope VALUES(?,?,?,?,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
            params![envelope, kind.key(), origin.to_string(), batch],
        )?;
        tx.execute(
            "INSERT INTO envelope_source VALUES(?,?,0,?)",
            params![envelope, source, duplicate],
        )?;
        tx.commit()?;
        Ok(IntakeOutcome { item, duplicate })
    }
    /// Progress of a dump batch across its envelopes.
    pub fn batch_progress(&self, batch: &str) -> Result<BatchProgress> {
        let (label, calls, allowance, requests): (String, i64, i64, i64) = self.db.query_row(
            "SELECT label,openai_calls,openai_allowance,typesafe_requests FROM import_batch WHERE id=?",
            [batch],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        let (files, duplicates, finished, failed): (i64, i64, i64, i64) = self.db.query_row("SELECT count(DISTINCT es.source_id),coalesce(sum(es.duplicate),0),count(DISTINCT CASE WHEN e.state IN ('done','failed','manual') OR p.graph_state IS NOT NULL THEN es.source_id END),count(DISTINCT CASE WHEN e.state='failed' THEN es.source_id END) FROM intake_envelope v JOIN envelope_source es ON es.envelope_id=v.id LEFT JOIN document_evaluation e ON e.source_id=es.source_id LEFT JOIN document_profile p ON p.source_id=es.source_id WHERE v.batch_id=?",[batch],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
        Ok(BatchProgress {
            id: batch.into(),
            label,
            files: files as usize,
            duplicates: duplicates as usize,
            finished: finished as usize,
            failed: failed as usize,
            openai_calls: calls,
            openai_allowance: allowance,
            typesafe_requests: requests,
        })
    }
    /// The batch an item arrived with most recently, if any.
    pub fn item_batch(&self, item: u64) -> Result<Option<String>> {
        Ok(self.db.query_row("SELECT v.batch_id FROM intake_envelope v JOIN envelope_source es ON es.envelope_id=v.id JOIN collection_item i ON i.source_id=es.source_id WHERE i.local_id=? AND v.batch_id IS NOT NULL ORDER BY v.received_at DESC LIMIT 1",[sql_id(item)?],|r|r.get(0)).optional()?)
    }
    pub fn record_batch_usage(
        &mut self,
        batch: &str,
        typesafe_requests: u32,
        typesafe_tokens: u64,
        openai_calls: u32,
    ) -> Result<()> {
        self.db.execute("UPDATE import_batch SET typesafe_requests=typesafe_requests+?,typesafe_input_tokens=typesafe_input_tokens+?,openai_calls=openai_calls+? WHERE id=?",params![typesafe_requests,typesafe_tokens as i64,openai_calls,batch])?;
        Ok(())
    }
    /// Reserves one OpenAI gap-fill call against the batch allowance.
    pub fn reserve_batch_openai_call(&mut self, batch: &str) -> Result<bool> {
        Ok(self.db.execute(
            "UPDATE import_batch SET openai_calls=openai_calls+1 WHERE id=? AND openai_calls<openai_allowance",
            [batch],
        )? == 1)
    }
    pub fn extend_batch_allowance(&mut self, batch: &str) -> Result<()> {
        self.db.execute(
            "UPDATE import_batch SET openai_allowance=openai_allowance+? WHERE id=?",
            params![DEFAULT_BATCH_OPENAI_ALLOWANCE, batch],
        )?;
        Ok(())
    }
    /// Section texts of a personal document, ordered, for the graph pipeline.
    pub fn document_texts(&self, item: u64) -> Result<(String, String, Vec<crate::SourceText>)> {
        let (source, title): (String, String) = self.db.query_row("SELECT s.id,s.title FROM collection_item i JOIN source s ON s.id=i.source_id WHERE i.local_id=? AND i.kind='document' AND i.deleted_at IS NULL AND s.sensitivity='personal' AND s.retention='keep'",[sql_id(item)?],|r|Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or(Error::Validation("Release the document content first."))?;
        let segments = self
            .db
            .prepare("SELECT id,ordinal,text FROM source_segment WHERE source_id=? AND ordinal>0 ORDER BY ordinal")?
            .query_map([&source], |r| {
                Ok(crate::SourceText {
                    segment_id: r.get(0)?,
                    ordinal: r.get(1)?,
                    text: r.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok((source, title, segments))
    }
    /// Lazy documents for an on-demand search: (source, title, excerpt).
    pub fn lazy_document_excerpts(&self, limit: usize) -> Result<Vec<(String, String, String)>> {
        Ok(self.db.prepare("SELECT p.source_id,s.title,coalesce((SELECT substr(g.text,1,2000) FROM source_segment g WHERE g.source_id=p.source_id AND g.ordinal>0 ORDER BY g.ordinal LIMIT 1),'') FROM document_profile p JOIN source s ON s.id=p.source_id WHERE p.tier='lazy' AND p.family<>'noise' AND s.retention='keep' ORDER BY p.classified_at DESC LIMIT ?")?
            .query_map([limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<std::result::Result<_, _>>()?)
    }
    pub fn lazy_document_count(&self) -> Result<usize> {
        Ok(self.db.query_row(
            "SELECT count(*) FROM document_profile p JOIN source s ON s.id=p.source_id WHERE p.tier='lazy' AND p.family<>'noise' AND s.retention='keep'",
            [],
            |r| r.get::<_, i64>(0),
        )? as usize)
    }
    /// Collection item of a source, for opening or deep analysis.
    pub fn source_item(&self, source: &str) -> Result<Option<u64>> {
        Ok(self
            .db
            .query_row(
                "SELECT local_id FROM collection_item WHERE source_id=? AND kind='document' AND deleted_at IS NULL",
                [source],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .map(|i| i as u64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_plans_skip_hidden_links_unsupported_and_empty_files() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("dump");
        std::fs::create_dir_all(root.join("Steuer/2025")).unwrap();
        std::fs::write(root.join("Steuer/2025/bescheid.pdf"), b"%PDF synthetic").unwrap();
        std::fs::write(root.join("pass.jpg"), b"synthetic jpeg").unwrap();
        std::fs::write(root.join(".DS_Store"), b"x").unwrap();
        std::fs::write(root.join("movie.mkv"), b"x").unwrap();
        std::fs::write(root.join("empty.pdf"), b"").unwrap();
        std::os::unix::fs::symlink(root.join("pass.jpg"), root.join("link.jpg")).unwrap();
        let plan = plan_folder_import(std::slice::from_ref(&root));
        assert_eq!(
            plan.files,
            vec![root.join("Steuer/2025/bescheid.pdf"), root.join("pass.jpg")]
        );
        assert_eq!((plan.unsupported, plan.hidden, plan.skipped), (1, 1, 2));
        assert!(!plan.truncated);
    }

    #[test]
    fn identical_content_is_stored_once_and_linked_to_each_envelope() {
        let t = tempfile::tempdir().unwrap();
        let mut v = Vault::create(&t.path().join("vault"), "synthetic-intake-password").unwrap();
        let a = t.path().join("a.txt");
        let b = t.path().join("copy of a.txt");
        std::fs::write(&a, "SYNTHETIC Versicherungsschein 123").unwrap();
        std::fs::write(&b, "SYNTHETIC Versicherungsschein 123").unwrap();
        let batch = v.begin_import_batch("Setup dump").unwrap();
        let first = v
            .import_envelope_file(&a, SourceKind::Folder, Some(&batch))
            .unwrap();
        let second = v
            .import_envelope_file(&b, SourceKind::Folder, Some(&batch))
            .unwrap();
        assert!(!first.duplicate && second.duplicate);
        assert_eq!(first.item, second.item);
        let progress = v.batch_progress(&batch).unwrap();
        assert_eq!((progress.files, progress.duplicates), (1, 1));
        assert_eq!(
            v.item_batch(first.item).unwrap().as_deref(),
            Some(batch.as_str())
        );
        // Bulk intake queues behind interactive drops.
        let drop = t.path().join("drop.txt");
        std::fs::write(&drop, "SYNTHETIC dropped").unwrap();
        let dropped = v
            .import_envelope_file(&drop, SourceKind::Drop, None)
            .unwrap();
        assert_eq!(v.next_automatic_document().unwrap(), Some(dropped.item));
        assert!(v.reserve_batch_openai_call(&batch).unwrap());
        v.db.execute(
            "UPDATE import_batch SET openai_allowance=1 WHERE id=?",
            [&batch],
        )
        .unwrap();
        assert!(!v.reserve_batch_openai_call(&batch).unwrap());
        v.extend_batch_allowance(&batch).unwrap();
        assert!(v.reserve_batch_openai_call(&batch).unwrap());
    }
}
