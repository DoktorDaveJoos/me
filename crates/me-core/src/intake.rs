//! Envelope intake: every source (window drop, folder dump, later a scanner or mail
//! forwarding) produces envelopes of original files. Sources know nothing about
//! stages or AI. Identical content is stored once; a repeat links the existing source.
use crate::{
    DocumentClass, Error, Result, Vault, crypto,
    vault::{MAX_FILE, hash, id, sql_id},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::json;
use std::path::{Path, PathBuf};

/// Files per folder plan; a larger dump is imported in several runs.
pub const MAX_PLANNED_FILES: usize = 20_000;
/// Default OpenAI reader allowance of one dump batch (reader and audit calls of
/// all its files), extendable by the same amount.
pub const DEFAULT_BATCH_OPENAI_ALLOWANCE: i64 = 200;
const MAX_DEPTH: usize = 32;
/// Kept personal documents with text that no read has stored yet; documents
/// classified as noise are left out.
const UNREAD_DOCUMENTS: &str = "FROM collection_item i JOIN source s ON s.id=i.source_id LEFT JOIN document_profile p ON p.source_id=s.id WHERE i.kind='document' AND i.deleted_at IS NULL AND s.sensitivity='personal' AND s.retention='keep' AND coalesce(p.family,'')<>'noise' AND NOT EXISTS(SELECT 1 FROM read_summary r WHERE r.source_id=s.id) AND EXISTS(SELECT 1 FROM source_segment g WHERE g.source_id=s.id AND g.ordinal>0)";

/// Failed files stopped at the OpenAI allowance of the import that gates them (the
/// one each arrived with most recently): `(source_id, batch_id)`. `?1` is the
/// refusal message, which identifies such stops recorded before they were typed.
const STOPPED_AT_IMPORT_ALLOWANCE: &str = "WITH stopped AS (SELECT e.source_id,(SELECT v.batch_id FROM intake_envelope v JOIN envelope_source es ON es.envelope_id=v.id WHERE es.source_id=e.source_id AND v.batch_id IS NOT NULL ORDER BY v.received_at DESC LIMIT 1) AS batch_id FROM document_evaluation e JOIN import_progress p ON p.source_id=e.source_id JOIN collection_item i ON i.source_id=e.source_id AND i.kind='document' AND i.deleted_at IS NULL WHERE e.state='failed' AND (p.error_code='batch_budget' OR (p.error_code='budget' AND e.error_message=?1)))";

/// The batch a source arrived with most recently, if any.
pub(crate) fn source_batch(db: &Connection, source: &str) -> Result<Option<String>> {
    Ok(db.query_row("SELECT v.batch_id FROM intake_envelope v JOIN envelope_source es ON es.envelope_id=v.id WHERE es.source_id=? AND v.batch_id IS NOT NULL ORDER BY v.received_at DESC LIMIT 1",[source],|r|r.get(0)).optional()?)
}
/// Charges one OpenAI call to a batch; `false` when its allowance is used up.
pub(crate) fn charge_batch_openai_call(db: &Connection, batch: &str) -> Result<bool> {
    Ok(db.execute(
        "UPDATE import_batch SET openai_calls=openai_calls+1 WHERE id=? AND openai_calls<openai_allowance",
        [batch],
    )? == 1)
}

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

/// An import whose OpenAI allowance is used up and has stopped files it gates.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ExhaustedBatch {
    pub id: String,
    pub label: String,
    pub openai_calls: i64,
    pub openai_allowance: i64,
    /// Files that stopped at this import's allowance and wait for more calls.
    pub stopped: usize,
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
    /// Imports whose OpenAI allowance is used up and that stopped at least one of
    /// their files, newest first. Read from the vault, so an allowance stop stays
    /// actionable after a restart or a later import.
    pub fn exhausted_import_batches(&self) -> Result<Vec<ExhaustedBatch>> {
        Ok(self.db.prepare(&format!("{STOPPED_AT_IMPORT_ALLOWANCE} SELECT b.id,b.label,b.openai_calls,b.openai_allowance,count(DISTINCT s.source_id) FROM import_batch b JOIN stopped s ON s.batch_id=b.id WHERE b.openai_calls>=b.openai_allowance GROUP BY b.id ORDER BY b.created_at DESC"))?
            .query_map([crate::import_recovery::IMPORT_ALLOWANCE_REACHED], |r| {
                Ok(ExhaustedBatch {
                    id: r.get(0)?,
                    label: r.get(1)?,
                    openai_calls: r.get(2)?,
                    openai_allowance: r.get(3)?,
                    stopped: r.get::<_, i64>(4)? as usize,
                })
            })?
            .collect::<std::result::Result<_, _>>()?)
    }
    /// Allows more OpenAI calls for a batch. Its files that stopped at its
    /// allowance are queued again while automatic analysis is on, so reading
    /// continues where it stopped; with it off they wait to be started, as a new
    /// import would. A file stopped at its own allowance keeps waiting for that
    /// file's extension.
    pub fn extend_batch_allowance(&mut self, batch: &str) -> Result<()> {
        let tx = self.db.transaction()?;
        tx.execute(
            "UPDATE import_batch SET openai_allowance=openai_allowance+? WHERE id=?",
            params![DEFAULT_BATCH_OPENAI_ALLOWANCE, batch],
        )?;
        tx.execute(
            &format!("{STOPPED_AT_IMPORT_ALLOWANCE} UPDATE document_evaluation SET state=CASE WHEN (SELECT automatic_evaluation FROM app_settings WHERE singleton=1)=1 THEN 'queued' ELSE 'manual' END,error_message=NULL WHERE state='failed' AND source_id IN (SELECT source_id FROM stopped WHERE batch_id=?2)"),
            params![crate::import_recovery::IMPORT_ALLOWANCE_REACHED, batch],
        )?;
        tx.commit()?;
        Ok(())
    }
    /// Section texts of a personal document, ordered, for the read pipeline.
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
    /// Documents not read yet, newest first, for an on-demand search: (source,
    /// title, excerpt). Only kept personal documents with text; noise is left out.
    pub fn unread_document_excerpts(&self, limit: usize) -> Result<Vec<(String, String, String)>> {
        Ok(self.db.prepare(&format!("SELECT s.id,s.title,coalesce((SELECT substr(g.text,1,2000) FROM source_segment g WHERE g.source_id=s.id AND g.ordinal>0 ORDER BY g.ordinal LIMIT 1),'') {UNREAD_DOCUMENTS} ORDER BY i.local_id DESC LIMIT ?"))?
            .query_map([limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<std::result::Result<_, _>>()?)
    }
    /// Documents that have no read yet (no read summary).
    pub fn unread_document_count(&self) -> Result<usize> {
        Ok(self.db.query_row(
            &format!("SELECT count(DISTINCT s.id) {UNREAD_DOCUMENTS}"),
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
        // New vaults start with automatic analysis off; these tests exercise the queue.
        v.set_automatic_evaluation(true).unwrap();
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
        // A file's usage names the import it arrived with and that import's allowance.
        let allowance = v.import_usage(first.item).unwrap().batch.unwrap();
        assert_eq!(
            (allowance.id.as_str(), allowance.openai_calls),
            (batch.as_str(), 0)
        );
        assert_eq!(allowance.openai_allowance, DEFAULT_BATCH_OPENAI_ALLOWANCE);
        // Bulk intake queues behind interactive drops.
        let drop = t.path().join("drop.txt");
        std::fs::write(&drop, "SYNTHETIC dropped").unwrap();
        let dropped = v
            .import_envelope_file(&drop, SourceKind::Drop, None)
            .unwrap();
        assert_eq!(v.next_automatic_document().unwrap(), Some(dropped.item));
        assert!(v.import_usage(dropped.item).unwrap().batch.is_none());
        assert!(charge_batch_openai_call(&v.db, &batch).unwrap());
        v.db.execute(
            "UPDATE import_batch SET openai_allowance=1 WHERE id=?",
            [&batch],
        )
        .unwrap();
        assert!(!charge_batch_openai_call(&v.db, &batch).unwrap());
        assert!(
            v.import_usage(first.item)
                .unwrap()
                .batch
                .unwrap()
                .exhausted()
        );
        v.extend_batch_allowance(&batch).unwrap();
        assert!(charge_batch_openai_call(&v.db, &batch).unwrap());
    }

    #[test]
    fn documents_without_a_read_are_offered_to_deep_search() {
        let t = tempfile::tempdir().unwrap();
        let mut v = Vault::create(&t.path().join("vault"), "synthetic-intake-password").unwrap();
        let mut sources = Vec::new();
        for (name, text) in [
            ("read.txt", "SYNTHETIC Versicherungsschein 123"),
            ("unread.txt", "SYNTHETIC Mietvertrag 456"),
            ("noise.txt", "SYNTHETIC Werbung 789"),
        ] {
            let file = t.path().join(name);
            std::fs::write(&file, text).unwrap();
            let item = v
                .import_document(&file, name, DocumentClass::Personal)
                .unwrap();
            sources.push(v.document_texts(item).unwrap().0);
        }
        assert_eq!(v.unread_document_count().unwrap(), 3);
        v.apply_read(
            &sources[0],
            &crate::DocumentRead {
                run_id: "synthetic-run".into(),
                ..Default::default()
            },
        )
        .unwrap();
        v.save_document_profile(&sources[2], "noise", 0.9, None, None, None, None, None)
            .unwrap();
        assert_eq!(v.unread_document_count().unwrap(), 1);
        let unread = v.unread_document_excerpts(10).unwrap();
        assert_eq!(unread.len(), 1);
        assert_eq!(
            (unread[0].0.as_str(), unread[0].1.as_str()),
            (sources[1].as_str(), "unread.txt")
        );
        assert!(unread[0].2.contains("Mietvertrag"));
    }

    #[test]
    fn reader_calls_are_bounded_by_the_import_allowance() {
        use crate::{ImportErrorKind, ImportFailure, ImportProvider};
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("vault");
        let mut v = Vault::create(&root, "synthetic-intake-password").unwrap();
        // New vaults start with automatic analysis off; these tests exercise the queue.
        v.set_automatic_evaluation(true).unwrap();
        let file = t.path().join("letter.txt");
        std::fs::write(&file, "SYNTHETIC Erika Beispiel\nSteuer-ID: 01234567890").unwrap();
        let item = v
            .import_document(&file, "letter.txt", DocumentClass::Personal)
            .unwrap();
        let batch = v.begin_import_batch("Setup dump").unwrap();
        let linked = v
            .import_envelope_file(&file, SourceKind::Folder, Some(&batch))
            .unwrap();
        assert!(linked.duplicate && linked.item == item);
        v.db.execute(
            "UPDATE import_batch SET openai_allowance=1 WHERE id=?",
            [&batch],
        )
        .unwrap();
        v.begin_evaluation(item, false).unwrap();
        v.begin_import_progress(item, "a").unwrap();
        let input = v.prepare_extraction(item, "synthetic").unwrap();
        v.reserve_import_request(&input, ImportProvider::OpenAi)
            .unwrap();
        let refused = v
            .reserve_import_request(&input, ImportProvider::OpenAi)
            .unwrap_err();
        // Every reservation path reports it as the import's allowance stop, not the
        // file's and not a broken source.
        assert_eq!(
            ImportFailure::refused_reservation(&refused).kind,
            ImportErrorKind::BatchBudget
        );
        let refused = refused.to_string();
        assert!(refused.contains("import") && refused.contains("allowance"));
        // The refused call is charged neither to the file nor to the import.
        assert_eq!(v.import_usage(item).unwrap().openai_calls, 1);
        // Every TypeSafe request counts toward the import: reader and pipeline alike,
        // with its reported input tokens once.
        let call = v
            .reserve_import_request(&input, ImportProvider::TypeSafe)
            .unwrap();
        v.reserve_graph_request(item, "a", ImportProvider::TypeSafe)
            .unwrap();
        v.record_import_usage(item, "a", &call, 100, 20).unwrap();
        v.record_import_usage(item, "a", &call, 100, 20).unwrap();
        let progress = v.batch_progress(&batch).unwrap();
        assert_eq!(
            (
                progress.openai_calls,
                progress.openai_allowance,
                progress.typesafe_requests
            ),
            (1, 1, 2)
        );
        let tokens: i64 =
            v.db.query_row(
                "SELECT typesafe_input_tokens FROM import_batch WHERE id=?",
                [&batch],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tokens, 100);

        // The file is left not read. Its stop names the import that refused the
        // call, and that import is listed as used up, also after a restart.
        let failure =
            ImportFailure::new(ImportProvider::Local, ImportErrorKind::BatchBudget, refused);
        v.record_import_failure(item, "a", &failure).unwrap();
        v.fail_extraction(&input.run_id).unwrap();
        v.finish_import_evaluation(item, "a", Some(&failure.message), None)
            .unwrap();
        drop(v);
        let mut v = Vault::unlock(&root, "synthetic-intake-password").unwrap();
        assert_eq!(v.next_automatic_document().unwrap(), None);
        let job = v
            .import_jobs()
            .unwrap()
            .into_iter()
            .find(|j| j.item == item)
            .unwrap();
        assert_eq!(job.failure.unwrap().kind, ImportErrorKind::BatchBudget);
        let gate = job.usage.batch.unwrap();
        assert_eq!((gate.id.as_str(), gate.exhausted()), (batch.as_str(), true));
        let stopped = v.exhausted_import_batches().unwrap();
        assert_eq!(stopped.len(), 1);
        assert_eq!(
            (stopped[0].id.as_str(), stopped[0].stopped),
            (batch.as_str(), 1)
        );
        // Extending that import queues the file again and lets its next call pass.
        v.extend_batch_allowance(&batch).unwrap();
        assert!(v.exhausted_import_batches().unwrap().is_empty());
        assert_eq!(v.next_automatic_document().unwrap(), Some(item));
        assert!(v.begin_evaluation(item, true).unwrap());
        v.begin_import_progress(item, "b").unwrap();
        let input = v.prepare_extraction(item, "synthetic").unwrap();
        v.reserve_import_request(&input, ImportProvider::OpenAi)
            .unwrap();
        assert_eq!(v.batch_progress(&batch).unwrap().openai_calls, 2);
    }

    #[test]
    fn an_exhausted_import_stops_its_files_before_any_paid_call() {
        use crate::{ImportErrorKind, ImportFailure, ImportProvider};
        let t = tempfile::tempdir().unwrap();
        let mut v = Vault::create(&t.path().join("vault"), "synthetic-intake-password").unwrap();
        v.set_automatic_evaluation(true).unwrap();
        let batch = v.begin_import_batch("Setup dump").unwrap();
        let mut items = Vec::new();
        for name in ["waiting.txt", "own-limit.txt", "earlier.txt"] {
            let file = t.path().join(name);
            std::fs::write(&file, format!("SYNTHETIC {name}")).unwrap();
            let outcome = v
                .import_envelope_file(&file, SourceKind::Folder, Some(&batch))
                .unwrap();
            items.push(outcome.item);
        }
        let loose = t.path().join("loose.txt");
        std::fs::write(&loose, "SYNTHETIC loose").unwrap();
        let loose = v
            .import_envelope_file(&loose, SourceKind::Drop, None)
            .unwrap()
            .item;
        let (waiting, own_limit, earlier) = (items[0], items[1], items[2]);
        assert!(v.ensure_batch_allowance(waiting).is_ok());
        // Other files of the import used its last OpenAI call.
        v.db.execute(
            "UPDATE import_batch SET openai_allowance=1,openai_calls=1 WHERE id=?",
            [&batch],
        )
        .unwrap();
        let before = v.batch_progress(&batch).unwrap();
        let refused = v.ensure_batch_allowance(waiting).unwrap_err();
        let failure = ImportFailure::refused_reservation(&refused);
        assert_eq!(
            (failure.provider, failure.kind),
            (ImportProvider::Local, ImportErrorKind::BatchBudget)
        );
        // The check reserves and charges nothing, to the file or to the import.
        let usage = v.import_usage(waiting).unwrap();
        assert_eq!((usage.openai_calls, usage.typesafe_calls), (0, 0));
        assert_eq!(v.batch_progress(&batch).unwrap(), before);
        // A file that came without an import is never stopped by it.
        assert!(v.ensure_batch_allowance(loose).is_ok());

        // One file stops at the import's allowance, another at its own. A third
        // stopped at the import's allowance before such stops had their own kind.
        let own = ImportFailure::new(
            ImportProvider::Local,
            ImportErrorKind::Budget,
            "SYNTHETIC file allowance",
        );
        let untyped = ImportFailure::new(
            ImportProvider::Local,
            ImportErrorKind::Budget,
            crate::import_recovery::IMPORT_ALLOWANCE_REACHED,
        );
        for (item, run, stop) in [
            (waiting, "w", &failure),
            (own_limit, "o", &own),
            (earlier, "e", &untyped),
        ] {
            v.begin_evaluation(item, false).unwrap();
            v.begin_import_progress(item, run).unwrap();
            v.record_import_failure(item, run, stop).unwrap();
            v.finish_import_evaluation(item, run, Some(&stop.message), None)
                .unwrap();
        }
        assert_eq!(v.exhausted_import_batches().unwrap()[0].stopped, 2);
        // Extending the import resumes only the file it stopped; the file at its
        // own limit keeps its own "allow more" action.
        v.extend_batch_allowance(&batch).unwrap();
        assert!(v.ensure_batch_allowance(waiting).is_ok());
        let state = |v: &Vault, item: u64| {
            v.import_jobs()
                .unwrap()
                .into_iter()
                .find(|j| j.item == item)
                .unwrap()
                .state
        };
        assert_eq!(state(&v, waiting), "queued");
        assert_eq!(state(&v, earlier), "queued");
        assert_eq!(state(&v, own_limit), "failed");
    }

    #[test]
    fn extending_an_import_queues_its_stopped_files_only_while_automatic_analysis_is_on() {
        use crate::{ImportErrorKind, ImportFailure, ImportProvider};
        for automatic in [true, false] {
            let t = tempfile::tempdir().unwrap();
            let mut v =
                Vault::create(&t.path().join("vault"), "synthetic-intake-password").unwrap();
            v.set_automatic_evaluation(automatic).unwrap();
            let batch = v.begin_import_batch("Setup dump").unwrap();
            let file = t.path().join("stopped.txt");
            std::fs::write(&file, "SYNTHETIC stopped").unwrap();
            let item = v
                .import_envelope_file(&file, SourceKind::Folder, Some(&batch))
                .unwrap()
                .item;
            let stop = ImportFailure::new(
                ImportProvider::Local,
                ImportErrorKind::BatchBudget,
                "SYNTHETIC import allowance",
            );
            v.begin_evaluation(item, false).unwrap();
            v.begin_import_progress(item, "a").unwrap();
            v.record_import_failure(item, "a", &stop).unwrap();
            v.finish_import_evaluation(item, "a", Some(&stop.message), None)
                .unwrap();
            v.extend_batch_allowance(&batch).unwrap();
            let state = v
                .import_jobs()
                .unwrap()
                .into_iter()
                .find(|j| j.item == item)
                .unwrap()
                .state;
            if automatic {
                assert_eq!(state, "queued");
                assert_eq!(v.next_automatic_document().unwrap(), Some(item));
            } else {
                // With automatic analysis off the file waits for the person, like a
                // new import, and never enters the automatic queue; a manual start
                // works right away.
                assert_eq!(state, "manual");
                assert_eq!(v.next_automatic_document().unwrap(), None);
                assert!(v.begin_evaluation(item, false).unwrap());
            }
        }
    }
}
