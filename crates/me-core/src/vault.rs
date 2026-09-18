use crate::{
    Collection, Content, DocumentClass, Error, Item, Result, Revision,
    crypto::{self, Header, Keys},
};
use fs2::FileExt;
use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, params};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    time::Duration,
};
use zeroize::Zeroizing;

const MAX_FILE: u64 = 64 * 1024 * 1024;
const MAX_TEXT: usize = 1024 * 1024;
const SCHEMA: &str = include_str!("../migrations/001_vault.sql");

/// Owns the only database connection and keys. Keep on a worker, never in an
/// agent's process or working directory. Dropping releases the session and lock.
pub struct Vault {
    pub(crate) db: Connection,
    root: PathBuf,
    header: Header,
    keys: Keys,
    _lock: File,
}

pub(crate) fn sql_id(id: u64) -> Result<i64> {
    i64::try_from(id).map_err(|_| Error::Validation("Invalid item ID."))
}
pub(crate) fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
pub(crate) fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn private_dir(path: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().mode(0o700).create(path)?;
    Ok(())
}
fn regular(path: &Path) -> Result<()> {
    if !fs::symlink_metadata(path)?.file_type().is_file() {
        return Err(Error::Format);
    }
    Ok(())
}
fn lock(root: &Path) -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = root.join("session.lock");
    if path.symlink_metadata().is_ok() {
        regular(&path)?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)?;
    file.try_lock_exclusive().map_err(|_| Error::InUse)?;
    Ok(file)
}
fn database(path: &Path, key: &[u8], create: bool) -> Result<Connection> {
    let mut flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    if create {
        flags |= OpenFlags::SQLITE_OPEN_CREATE;
    } else {
        regular(path)?;
    }
    let db = Connection::open_with_flags(path, flags)?;
    let key = Zeroizing::new(format!("x'{}'", hex::encode(key)));
    db.pragma_update(None, "key", key.as_str())?;
    let cipher: String = db.query_row("PRAGMA cipher_version", [], |row| row.get(0))?;
    if cipher.is_empty() {
        return Err(Error::Format);
    }
    db.query_row("SELECT count(*) FROM sqlite_master", [], |row| {
        row.get::<_, i64>(0)
    })
    .map_err(|_| Error::Authentication)?;
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA temp_store=MEMORY; PRAGMA cipher_memory_security=ON; PRAGMA secure_delete=ON; PRAGMA synchronous=FULL; PRAGMA journal_mode=DELETE;")?;
    db.busy_timeout(Duration::from_secs(2))?;
    Ok(db)
}

impl Vault {
    pub fn exists(root: &Path) -> Result<bool> {
        match fs::symlink_metadata(root) {
            Ok(meta) if meta.file_type().is_dir() => {
                regular(&root.join("header.json"))?;
                regular(&root.join("vault.db"))?;
                Ok(true)
            }
            Ok(_) => Err(Error::Format),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err.into()),
        }
    }

    pub fn create(root: &Path, password: &str) -> Result<Self> {
        let (header, keys) = Header::create(password)?;
        if let Some(parent) = root.parent() {
            fs::create_dir_all(parent)?;
        }
        private_dir(root)?; // exclusive; never erase or reuse an existing vault
        let result = (|| {
            let guard = lock(root)?;
            private_dir(&root.join("objects"))?;
            let mut db = database(&root.join("vault.db"), &keys[..32], true)?;
            let tx = db.transaction()?;
            tx.execute_batch(SCHEMA)?;
            tx.execute_batch(include_str!("../migrations/002_ai.sql"))?;
            tx.execute_batch(include_str!("../migrations/003_inbox_settings.sql"))?;
            tx.execute_batch(include_str!("../migrations/004_extraction_checkpoints.sql"))?;
            tx.execute_batch(include_str!("../migrations/005_review_questions.sql"))?;
            tx.execute_batch(include_str!("../migrations/006_credentials.sql"))?;
            tx.execute_batch(include_str!("../migrations/007_browser.sql"))?;
            tx.execute_batch(include_str!("../migrations/008_data_recent.sql"))?;
            tx.execute_batch(include_str!("../migrations/009_import_progress.sql"))?;
            let profile = id();
            tx.execute("INSERT INTO entity(id,kind,label,created_at) VALUES(?,'person','Ich',strftime('%Y-%m-%dT%H:%M:%fZ','now'))", [&profile])?;
            tx.execute(
                "INSERT INTO vault_meta VALUES(1,?,?,?)",
                params![header.vault_id, id(), profile],
            )?;
            tx.commit()?;
            header.write(&root.join("header.json"))?;
            File::open(root)?.sync_all()?;
            Ok(Self {
                db,
                root: root.to_owned(),
                header,
                keys,
                _lock: guard,
            })
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(root);
        }
        result
    }

    pub fn unlock(root: &Path, password: &str) -> Result<Self> {
        if !Self::exists(root)? {
            return Err(Error::Validation("No vault found here."));
        }
        let guard = lock(root)?;
        let header = Header::read(&root.join("header.json"))?;
        let keys = header.unlock(password)?;
        let mut db = database(&root.join("vault.db"), &keys[..32], false)?;
        let version: i64 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if !(1..=9).contains(&version) {
            return Err(Error::Format);
        }
        if version == 1 {
            let tx = db.transaction()?;
            tx.execute_batch(include_str!("../migrations/002_ai.sql"))?;
            tx.commit()?;
        }
        if version < 3 {
            let tx = db.transaction()?;
            tx.execute_batch(include_str!("../migrations/003_inbox_settings.sql"))?;
            tx.commit()?;
        }
        if version < 4 {
            let tx = db.transaction()?;
            tx.execute_batch(include_str!("../migrations/004_extraction_checkpoints.sql"))?;
            tx.commit()?;
        }
        if version < 5 {
            let tx = db.transaction()?;
            tx.execute_batch(include_str!("../migrations/005_review_questions.sql"))?;
            tx.commit()?;
        }
        if version < 6 {
            let tx = db.transaction()?;
            tx.execute_batch(include_str!("../migrations/006_credentials.sql"))?;
            tx.commit()?;
        }
        if version < 7 {
            let tx = db.transaction()?;
            tx.execute_batch(include_str!("../migrations/007_browser.sql"))?;
            tx.commit()?;
        }
        if version < 8 {
            let tx = db.transaction()?;
            tx.execute_batch(include_str!("../migrations/008_data_recent.sql"))?;
            tx.commit()?;
        }
        if version < 9 {
            let tx = db.transaction()?;
            tx.execute_batch(include_str!("../migrations/009_import_progress.sql"))?;
            tx.commit()?;
        }
        let vault_id: String =
            db.query_row("SELECT vault_id FROM vault_meta", [], |row| row.get(0))?;
        if vault_id != header.vault_id {
            return Err(Error::Authentication);
        }
        let mut vault = Self {
            db,
            root: root.to_owned(),
            header,
            keys,
            _lock: guard,
        };
        vault.verify()?;
        vault.db.execute("UPDATE job SET state='failed',last_error_code='interrupted' WHERE kind='index_text' AND state IN ('queued','running') AND source_id IN (SELECT source_id FROM collection_item WHERE extension NOT IN ('txt','md','csv'))", [])?;
        vault.db.execute("UPDATE job SET state='queued' WHERE kind='index_text' AND state='running' AND source_id IN (SELECT source_id FROM collection_item WHERE extension IN ('txt','md','csv'))", [])?;
        vault.resume_jobs()?;
        vault.db.execute("UPDATE job SET state='waiting_provider',last_error_code='interrupted' WHERE kind='extract_facts' AND state='running'", [])?;
        vault.db.execute(
            "UPDATE extraction_run SET status='failed' WHERE status='running'",
            [],
        )?;
        vault.db.execute("UPDATE document_evaluation SET state=CASE WHEN (SELECT automatic_evaluation FROM app_settings WHERE singleton=1)=1 THEN 'queued' ELSE 'manual' END,error_message=NULL WHERE state='running'", [])?;
        Ok(vault)
    }

    pub fn collection(&self, query: &str, pinned: bool) -> Result<Collection> {
        if query.len() > 4096 {
            return Err(Error::Validation("This search query is too long."));
        }
        let terms: Vec<_> = query
            .split(|ch: char| !ch.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .take(32)
            .collect();
        let fts = terms
            .iter()
            .map(|s| format!("\"{s}\"*"))
            .collect::<Vec<_>>()
            .join(" AND ");
        let total: i64 = self.db.query_row(
            "SELECT count(*) FROM collection_item WHERE deleted_at IS NULL",
            [],
            |r| r.get(0),
        )?;
        let pins: i64 = self.db.query_row(
            "SELECT count(*) FROM collection_item WHERE pinned=1 AND deleted_at IS NULL",
            [],
            |r| r.get(0),
        )?;
        let mut stmt = self.db.prepare("SELECT i.local_id,i.stable_id,i.title,i.kind,a.value_json,i.extension,i.pinned,CASE WHEN EXISTS(SELECT 1 FROM ai_question q WHERE q.source_id=i.source_id AND q.state='pending') THEN 'needs_answer' WHEN aj.state='needs_review' THEN 'needs_review' WHEN aj.state='done' THEN 'done' WHEN e.state='done' AND aj.state IS NULL THEN 'manual' ELSE coalesce(e.state,j.state,'manual') END,c.category,c.vault_name,c.archived
            FROM collection_item i LEFT JOIN assertion a ON a.id=i.current_assertion_id LEFT JOIN job j ON j.source_id=i.source_id AND j.kind='index_text' LEFT JOIN job aj ON aj.source_id=i.source_id AND aj.kind='extract_facts' LEFT JOIN document_evaluation e ON e.source_id=i.source_id LEFT JOIN credential_record c ON c.item_id=i.local_id
            WHERE i.deleted_at IS NULL AND (?1=0 OR i.pinned=1)
            AND (?2='' OR i.source_id IN (SELECT s.source_id FROM segment_fts JOIN source_segment s ON s.rowid=segment_fts.rowid WHERE segment_fts MATCH ?2) OR (i.kind='credential' AND instr(lower(i.title || ' ' || c.vault_name),lower(?3))>0))
            ORDER BY i.pinned DESC,i.local_id DESC LIMIT ?4")?;
        let items = stmt
            .query_map(
                params![
                    pinned,
                    fts,
                    query.trim(),
                    if fts.is_empty() { -1 } else { 200 }
                ],
                |r| {
                    let kind: String = r.get(3)?;
                    let content = if kind == "note" {
                        let raw: String = r.get(4)?;
                        Content::Note(serde_json::from_str(&raw).unwrap_or_default())
                    } else if kind == "credential" {
                        Content::Credential {
                            category: r.get(8)?,
                            vault: r.get(9)?,
                            archived: r.get(10)?,
                        }
                    } else {
                        Content::Document {
                            extension: r.get(5)?,
                            state: r.get(7)?,
                        }
                    };
                    Ok(Item {
                        id: r.get::<_, i64>(0)? as u64,
                        stable_id: r.get(1)?,
                        title: r.get(2)?,
                        content,
                        pinned: r.get(6)?,
                    })
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(Collection {
            items,
            total: total as usize,
            pins: pins as usize,
        })
    }

    /// A manual save is an explicit user decision. Extractors will use a separate
    /// proposal API; they must never call this path to impersonate confirmation.
    pub fn save_note(&mut self, item_id: Option<u64>, title: &str, value: &str) -> Result<u64> {
        let title = title.trim();
        let value = value.trim();
        if title.is_empty() || title.chars().count() > 200 {
            return Err(Error::Validation("Enter a label with 1 to 200 characters."));
        }
        if value.is_empty() || value.len() > 16000 {
            return Err(Error::Validation(
                "Enter a value or note up to 16,000 bytes.",
            ));
        }
        let selected = crate::standard_field(title);
        let tx = self.db.transaction()?;
        let profile: String =
            tx.query_row("SELECT profile_id FROM vault_meta", [], |r| r.get(0))?;
        let old: Option<(String, String, String, i64)> = if let Some(item_id) = item_id {
            Some(tx.query_row("SELECT stable_id,property_key,current_assertion_id,revision FROM collection_item WHERE local_id=? AND kind='note' AND deleted_at IS NULL", [sql_id(item_id)?], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?.ok_or(Error::Validation("This detail is no longer available."))?)
        } else {
            None
        };
        let stable = old.as_ref().map(|v| v.0.clone()).unwrap_or_else(id);
        let property = old.as_ref().map(|v| v.1.clone()).unwrap_or_else(|| {
            selected.map_or_else(|| format!("custom.{stable}"), |f| f.key.into())
        });
        let field = crate::STANDARD_FIELDS.iter().find(|f| f.key == property);
        let value = if let Some(field) = field {
            crate::normalize_fact(field.key, value)?
        } else {
            value.to_owned()
        };
        let value = value.as_str();
        let value_type = field.map_or("text", |f| f.value_type);
        let revision = old.as_ref().map_or(1, |v| v.3 + 1);
        tx.execute("INSERT OR IGNORE INTO property_definition VALUES(?,?,'text','one','explicit_user_intent','{}',1)", params![property, title])?;
        let source = id();
        tx.execute("INSERT INTO source(id,kind,title,received_at,content_fingerprint,sensitivity,retention) VALUES(?,'note',?,strftime('%Y-%m-%dT%H:%M:%fZ','now'),?,'personal','keep')", params![source,title,hash(value.as_bytes())])?;
        let raw = serde_json::to_string(value).map_err(|_| Error::Format)?;
        let semantic = hash(
            json!([profile, property, value, "unknown"])
                .to_string()
                .as_bytes(),
        );
        let existing: Option<String> = tx
            .query_row(
                "SELECT id FROM assertion WHERE semantic_key=?",
                [&semantic],
                |r| r.get(0),
            )
            .optional()?;
        let assertion = existing.unwrap_or_else(id);
        tx.execute("INSERT OR IGNORE INTO assertion(id,subject_id,property_key,value_type,value_json,canonical_value,semantic_key,time_kind,recorded_at,origin,supersedes_id)
            VALUES(?,?,?,?,?,?,?,'unknown',strftime('%Y-%m-%dT%H:%M:%fZ','now'),'user',?)", params![assertion,profile,property,value_type,raw,raw,semantic,old.as_ref().map(|v| &v.2)])?;
        if let Some(old) = &old {
            decision(&tx, &old.2, "retract")?;
        }
        decision(&tx, &assertion, "accept")?;
        tx.execute(
            "INSERT INTO assertion_evidence VALUES(?,?,?,'{\"kind\":\"manual\"}')",
            params![assertion, source, id()],
        )?;
        tx.execute(
            "INSERT INTO source_entity VALUES(?,?,'provided_by','accepted')",
            params![source, profile],
        )?;
        segment(&tx, &source, 0, &format!("{title}\n{value}"))?;
        let local_id = if let Some(item_id) = item_id {
            tx.execute("UPDATE collection_item SET title=?,source_id=?,current_assertion_id=?,revision=? WHERE local_id=?", params![title,source,assertion,revision,sql_id(item_id)?])?;
            item_id
        } else {
            tx.execute("INSERT INTO collection_item(stable_id,title,kind,source_id,property_key,current_assertion_id) VALUES(?,?,'note',?,?,?)",params![stable,title,source,property,assertion])?;
            tx.last_insert_rowid() as u64
        };
        journal(
            &tx,
            &stable,
            revision,
            "note_saved",
            json!({"source_id":source,"assertion_id":assertion}),
        )?;
        tx.commit()?;
        Ok(local_id)
    }

    pub fn history(&self, item: u64) -> Result<Vec<Revision>> {
        let mut stmt = self.db.prepare("SELECT a.value_json,a.state,a.recorded_at,(SELECT s.title FROM assertion_evidence e JOIN source s ON s.id=e.source_id WHERE e.assertion_id=a.id ORDER BY s.received_at DESC LIMIT 1) FROM collection_item i JOIN assertion_state a ON a.property_key=i.property_key WHERE i.local_id=? ORDER BY a.recorded_at DESC LIMIT 100")?;
        Ok(stmt
            .query_map([sql_id(item)?], |r| {
                let value: String = r.get(0)?;
                Ok(Revision {
                    value: serde_json::from_str(&value).unwrap_or_default(),
                    state: r.get(1)?,
                    recorded_at: r.get(2)?,
                    source_title: r.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn toggle_pin(&mut self, item: u64) -> Result<()> {
        let tx = self.db.transaction()?;
        let (stable,rev): (String,i64) = tx.query_row("UPDATE collection_item SET pinned=1-pinned,revision=revision+1 WHERE local_id=? AND deleted_at IS NULL RETURNING stable_id,revision",[sql_id(item)?],|r|Ok((r.get(0)?,r.get(1)?)))?;
        journal(&tx, &stable, rev, "pin_changed", json!({}))?;
        tx.commit()?;
        Ok(())
    }

    /// Delivery idempotency is independent of content equality. A caller retries
    /// a delivery with the same key; a deliberate second import uses a new key.
    pub fn import_document(
        &mut self,
        path: &Path,
        delivery_key: &str,
        class: DocumentClass,
    ) -> Result<u64> {
        if delivery_key.is_empty() || delivery_key.len() > 200 {
            return Err(Error::Validation("Invalid import ID."));
        }
        let existing: Option<i64> = self.db.query_row("SELECT i.local_id FROM inbox_item b JOIN inbox_source bs ON bs.inbox_id=b.id JOIN collection_item i ON i.source_id=bs.source_id WHERE b.connector_id='desktop-manual' AND b.delivery_key=?", [delivery_key], |r| r.get(0)).optional()?;
        if let Some(existing) = existing {
            return Ok(existing as u64);
        }
        if !crate::supported_document(path) {
            return Err(Error::Validation(
                "Supported files: PDFs, images, Office files, and text up to 64 MiB.",
            ));
        }
        let title = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or(Error::Validation("Invalid filename."))?;
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let bytes = crypto::read_bounded(path, MAX_FILE)?;
        let object = id();
        let source = id();
        let stable = id();
        let inbox = id();
        let ciphertext = crypto::seal(
            &self.keys[32..],
            &bytes,
            self.object_context(&object).as_bytes(),
        )?;
        let object_path = self.object_path(&object)?;
        crypto::atomic_write(&object_path, &ciphertext)?;
        let result = (|| {
            let tx = self.db.transaction()?;
            let sensitivity = match class {
                DocumentClass::Credential => "credential",
                DocumentClass::Personal => "personal",
                DocumentClass::Unclassified => "restricted",
            };
            tx.execute("INSERT INTO inbox_item VALUES(?,'manual','desktop-manual',?,strftime('%Y-%m-%dT%H:%M:%fZ','now'),'staged')",params![inbox,delivery_key])?;
            tx.execute("INSERT INTO source(id,kind,title,received_at,content_fingerprint,sensitivity,retention,object_id) VALUES(?,'file',?,strftime('%Y-%m-%dT%H:%M:%fZ','now'),?,?,'keep',?)",params![source,title,hash(&bytes),sensitivity,object])?;
            tx.execute(
                "INSERT INTO inbox_source VALUES(?,?)",
                params![inbox, source],
            )?;
            tx.execute("INSERT INTO collection_item(stable_id,title,kind,source_id,extension) VALUES(?,?,'document',?,?)",params![stable,title,source,extension])?;
            let local = tx.last_insert_rowid() as u64;
            tx.execute("INSERT INTO document_evaluation(source_id,state) VALUES(?,CASE WHEN ?!='credential' AND (SELECT automatic_evaluation FROM app_settings WHERE singleton=1)=1 THEN 'queued' ELSE 'manual' END)",params![source,sensitivity])?;
            // Filename-only search, separate from source text processing.
            if sensitivity != "credential" {
                segment(&tx, &source, 0, title)?;
            }
            let (state, error) = if matches!(class, DocumentClass::Personal)
                && ["txt", "md", "csv"].contains(&extension.as_str())
            {
                ("queued", None)
            } else {
                (
                    if matches!(class, DocumentClass::Personal) {
                        "waiting_extractor"
                    } else {
                        "needs_review"
                    },
                    Some("classification_or_extractor_required"),
                )
            };
            tx.execute("INSERT INTO job(id,source_id,kind,dedup_key,state,last_error_code) VALUES(?,?,'index_text',?,?,?)",params![id(),source,format!("text-v1:{source}"),state,error])?;
            tx.execute(
                "UPDATE inbox_item SET state='needs_review' WHERE id=?",
                [&inbox],
            )?;
            journal(
                &tx,
                &stable,
                1,
                "document_imported",
                json!({"source_id":source,"object_id":object,"inbox_id":inbox}),
            )?;
            tx.commit()?;
            Ok(local)
        })();
        if result.is_err() {
            let _ = fs::remove_file(object_path);
        }
        // Intake has committed before processing. Failed jobs stay visible and
        // can be retried after restart; an indexing error is not a lost import.
        if result.is_ok() {
            self.resume_jobs()?;
        }
        result
    }

    pub fn enable_text_search(&mut self, item: u64) -> Result<()> {
        let (source,extension,sensitivity): (String,String,String) = self.db.query_row("SELECT i.source_id,i.extension,s.sensitivity FROM collection_item i JOIN source s ON s.id=i.source_id WHERE i.local_id=? AND i.kind='document' AND i.deleted_at IS NULL", [sql_id(item)?], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        if sensitivity == "credential" {
            return Err(Error::Validation("Credentials are excluded from search."));
        }
        if !["txt", "md", "csv"].contains(&extension.as_str()) {
            return Err(Error::Validation(
                "Start document processing for this format.",
            ));
        }
        let indexed: bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM job WHERE source_id=? AND kind='index_text' AND state='done')",[&source],|r|r.get(0))?;
        if sensitivity == "personal" && indexed {
            return Ok(());
        }
        let tx = self.db.transaction()?;
        tx.execute(
            "UPDATE source SET sensitivity='personal' WHERE id=?",
            [&source],
        )?;
        tx.execute("UPDATE job SET state='queued',last_error_code=NULL WHERE source_id=? AND kind='index_text'",[&source])?;
        tx.commit()?;
        self.resume_jobs()?;
        let done: bool = self.db.query_row(
            "SELECT state='done' FROM job WHERE source_id=? AND kind='index_text'",
            [&source],
            |r| r.get(0),
        )?;
        if !done {
            return Err(Error::Validation(
                "Couldn't read the text. Use UTF-8 text up to 1 MiB.",
            ));
        }
        Ok(())
    }

    fn resume_jobs(&mut self) -> Result<()> {
        let sources = {
            let mut stmt = self.db.prepare("SELECT source_id FROM job WHERE kind='index_text' AND state='queued' AND source_id IN (SELECT source_id FROM collection_item WHERE extension IN ('txt','md','csv'))")?;
            stmt.query_map([], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        for source in sources {
            self.db.execute(
                "UPDATE job SET state='running',attempts=attempts+1 WHERE source_id=? AND kind='index_text'",
                [&source],
            )?;
            if self.index_text(&source).is_err() {
                self.db.execute("UPDATE job SET state='failed',last_error_code='text_index_failed' WHERE source_id=? AND kind='index_text'",[&source])?;
            }
        }
        Ok(())
    }

    fn index_text(&mut self, source: &str) -> Result<()> {
        let (object, sensitivity): (String, String) = self.db.query_row(
            "SELECT object_id,sensitivity FROM source WHERE id=?",
            [source],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if sensitivity != "personal" {
            return Err(Error::Validation("Content search hasn't been enabled."));
        }
        let bytes = self.read_object(&object)?;
        if bytes.len() > MAX_TEXT {
            return Err(Error::Validation(
                "Content search is limited to 1 MiB of text.",
            ));
        }
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| Error::Validation("Content search requires UTF-8 text."))?;
        if text.contains('\0') {
            return Err(Error::Validation("This file contains binary data."));
        }
        let tx = self.db.transaction()?;
        tx.execute(
            "DELETE FROM source_segment WHERE source_id=? AND ordinal>0",
            [source],
        )?;
        // Paragraph-sized chunks preserve exact text and a source-relative locator.
        let mut start = 0;
        let mut ordinal = 1;
        while start < text.len() {
            let mut end = (start + 1600).min(text.len());
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            segment(&tx, source, ordinal, &text[start..end])?;
            ordinal += 1;
            start = end;
        }
        tx.execute(
            "UPDATE source SET extraction_revision=1 WHERE id=?",
            [source],
        )?;
        tx.execute(
            "UPDATE job SET state='done',last_error_code=NULL WHERE source_id=? AND kind='index_text'",
            [source],
        )?;
        tx.execute("UPDATE inbox_item SET state='done' WHERE id IN (SELECT inbox_id FROM inbox_source WHERE source_id=?)",[source])?;
        tx.commit()?;
        Ok(())
    }

    fn object_path(&self, object: &str) -> Result<PathBuf> {
        let parsed = uuid::Uuid::parse_str(object).map_err(|_| Error::Format)?;
        if parsed.to_string() != object {
            return Err(Error::Format);
        }
        Ok(self.root.join("objects").join(object))
    }
    fn object_context(&self, object: &str) -> String {
        format!("me-object-v1:{}:{object}", self.header.vault_id)
    }
    pub(crate) fn read_object(&self, object: &str) -> Result<Zeroizing<Vec<u8>>> {
        let path = self.object_path(object)?;
        regular(&path)?;
        let data = crypto::read_bounded(&path, MAX_FILE + 48)?;
        crypto::open(
            &self.keys[32..],
            &data,
            self.object_context(object).as_bytes(),
        )
    }

    /// Explicit user export only. Never creates an automatic plaintext preview.
    pub fn export_document(&self, item: u64, destination: &Path) -> Result<()> {
        let object: String = self.db.query_row("SELECT s.object_id FROM collection_item i JOIN source s ON s.id=i.source_id WHERE i.local_id=? AND i.kind='document' AND i.deleted_at IS NULL",[sql_id(item)?],|r|r.get(0))?;
        let data = self.read_object(&object)?;
        crypto::atomic_write(destination, &data)
    }

    pub fn verify(&self) -> Result<()> {
        let integrity: String = self
            .db
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if integrity != "ok" {
            return Err(Error::Format);
        }
        if self.db.prepare("PRAGMA foreign_key_check")?.exists([])? {
            return Err(Error::Format);
        }
        // Object existence is cheap at unlock; full authentication is done on
        // read and for every object before publishing backup/restore.
        for object in self.objects()? {
            regular(&self.object_path(&object)?)?;
        }
        Ok(())
    }
    fn objects(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .db
            .prepare("SELECT DISTINCT object_id FROM source WHERE object_id IS NOT NULL")?;
        Ok(stmt
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Exclusive destination + encrypted SQLite backup API + immutable objects.
    /// Only header.json marks a complete backup; it is published last.
    pub fn backup(&self, destination: &Path) -> Result<()> {
        private_dir(destination)?;
        let result = (|| {
            private_dir(&destination.join("objects"))?;
            let mut copy = database(&destination.join("vault.db"), &self.keys[..32], true)?;
            rusqlite::backup::Backup::new(&self.db, &mut copy)?.run_to_completion(
                128,
                Duration::from_millis(5),
                None,
            )?;
            drop(copy);
            for object in self.objects()? {
                // Reject corruption before acknowledging the snapshot.
                let _plaintext = self.read_object(&object)?;
                let data = crypto::read_bounded(&self.object_path(&object)?, MAX_FILE + 48)?;
                crypto::atomic_write(&destination.join("objects").join(&object), &data)?;
            }
            self.header.write(&destination.join("header.json"))?;
            File::open(destination)?.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(destination);
        }
        result
    }

    pub fn restore(backup: &Path, destination: &Path, password: &str) -> Result<Self> {
        if destination.try_exists()? {
            return Err(Error::Validation("Restore requires a new vault folder."));
        }
        // A temporary working copy keeps the original backup immutable and
        // allows read-only backup media. Reject symlinks and unexpected files.
        let parent = destination.parent().ok_or(Error::Format)?;
        fs::create_dir_all(parent)?;
        let staging = tempfile::tempdir_in(parent)?;
        let staged = staging.path().join("vault");
        private_dir(&staged)?;
        private_dir(&staged.join("objects"))?;
        for name in ["header.json", "vault.db"] {
            regular(&backup.join(name))?;
            fs::copy(backup.join(name), staged.join(name))?;
        }
        if !fs::symlink_metadata(backup.join("objects"))?
            .file_type()
            .is_dir()
        {
            return Err(Error::Format);
        }
        for entry in fs::read_dir(backup.join("objects"))? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                return Err(Error::Format);
            }
            let name = entry.file_name();
            uuid::Uuid::parse_str(name.to_str().ok_or(Error::Format)?)
                .map_err(|_| Error::Format)?;
            fs::copy(entry.path(), staged.join("objects").join(name))?;
        }
        let vault = Self::unlock(&staged, password)?;
        vault.backup(destination)?;
        drop(vault);
        Self::unlock(destination, password)
    }
}

pub(crate) fn decision(tx: &Transaction<'_>, assertion: &str, action: &str) -> Result<()> {
    tx.execute("INSERT INTO decision(id,assertion_id,action,actor,reason_code,recorded_at) VALUES(?,?,?,'user','manual_save',strftime('%Y-%m-%dT%H:%M:%fZ','now'))",params![id(),assertion,action])?;
    Ok(())
}
pub(crate) fn segment(tx: &Transaction<'_>, source: &str, ordinal: i64, text: &str) -> Result<()> {
    let locator =
        json!({"kind":if ordinal==0 {"title_or_manual"}else{"text_chunk"},"ordinal":ordinal});
    tx.execute("INSERT INTO source_segment(id,source_id,extraction_revision,ordinal,locator_json,text,content_hash) VALUES(?,?,1,?,?,?,?)",params![id(),source,ordinal,locator.to_string(),text,hash(text.as_bytes())])?;
    Ok(())
}
pub(crate) fn journal(
    tx: &Transaction<'_>,
    stable: &str,
    revision: i64,
    kind: &str,
    payload: serde_json::Value,
) -> Result<()> {
    tx.execute("INSERT INTO local_change(operation_id,item_id,revision,kind,payload_json,recorded_at) VALUES(?,?,?,?,?,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",params![id(),stable,revision,kind,payload.to_string()])?;
    Ok(())
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    #[test]
    fn interrupted_index_job_is_replayed_atomically_on_unlock() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        let mut vault = Vault::create(&root, "synthetic-passphrase-2026").unwrap();
        let file = temp.path().join("sample.txt");
        fs::write(&file, "SYNTHETIC wiederaufnehmbar München").unwrap();
        let item = vault
            .import_document(&file, "delivery", DocumentClass::Personal)
            .unwrap();
        vault
            .db
            .execute("UPDATE job SET state='running'", [])
            .unwrap();
        vault
            .db
            .execute("DELETE FROM source_segment WHERE ordinal>0", [])
            .unwrap();
        drop(vault);
        let vault = Vault::unlock(&root, "synthetic-passphrase-2026").unwrap();
        assert_eq!(
            vault.collection("wiederaufnehmbar", false).unwrap().items[0].id,
            item
        );
        let state: String = vault
            .db
            .query_row("SELECT state FROM job", [], |r| r.get(0))
            .unwrap();
        assert_eq!(state, "done");
        assert_eq!(vault.collection("", false).unwrap().total, 1);
    }
    #[test]
    fn failed_transaction_has_no_partial_note_or_change_event() {
        let temp = tempfile::tempdir().unwrap();
        let mut vault =
            Vault::create(&temp.path().join("vault"), "synthetic-passphrase-2026").unwrap();
        vault.db.execute_batch("CREATE TRIGGER synthetic_failure BEFORE INSERT ON local_change BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
        assert!(vault.save_note(None, "Notiz", "SYNTHETIC").is_err());
        for table in [
            "source",
            "assertion",
            "decision",
            "collection_item",
            "local_change",
        ] {
            let count: i64 = vault
                .db
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0);
        }
    }
    #[test]
    fn schema_one_migration_preserves_existing_free_notes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        let mut vault = Vault::create(&root, "synthetic-passphrase-2026").unwrap();
        vault
            .save_note(None, "Legacy", "SYNTHETIC migration")
            .unwrap();
        vault.db.execute_batch("DROP TABLE import_progress; DROP TABLE data_recent; DROP TABLE document_folder; DROP TABLE credential_record; DROP TABLE credential_import; DROP TABLE ai_question; DROP TABLE extraction_checkpoint; DROP TABLE document_evaluation; DROP TABLE app_settings; DROP TABLE ai_proposal; DELETE FROM property_definition WHERE key LIKE 'person.%'; PRAGMA user_version=1;").unwrap();
        drop(vault);
        let vault = Vault::unlock(&root, "synthetic-passphrase-2026").unwrap();
        assert_eq!(vault.collection("migration", false).unwrap().total, 1);
        let version: i64 = vault
            .db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 9);
        let facts = vault
            .facts_get(&vault.shareable_scope().unwrap(), &["person.tax_id".into()])
            .unwrap();
        assert_eq!(facts["facts"][0]["status"], "missing");
    }
    #[test]
    fn schema_two_migration_queues_existing_documents_without_erasing_them() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        let password = "synthetic-passphrase-2026";
        let mut vault = Vault::create(&root, password).unwrap();
        let source = temp.path().join("test.pdf");
        fs::write(&source, "synthetic").unwrap();
        let item = vault
            .import_document(&source, "synthetic", DocumentClass::Unclassified)
            .unwrap();
        vault
            .db
            .execute_batch(
                "DROP TABLE import_progress; DROP TABLE data_recent; DROP TABLE document_folder; DROP TABLE credential_record; DROP TABLE credential_import; DROP TABLE ai_question; DROP TABLE extraction_checkpoint; DROP TABLE document_evaluation; DROP TABLE app_settings; PRAGMA user_version=2;",
            )
            .unwrap();
        drop(vault);
        let vault = Vault::unlock(&root, password).unwrap();
        assert!(vault.settings().unwrap().automatic_evaluation);
        assert_eq!(vault.next_automatic_document().unwrap(), Some(item));
        assert_eq!(vault.collection("", false).unwrap().total, 1);
    }

    #[test]
    fn schema_three_migration_preserves_manual_processing_and_existing_imports() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        let mut vault = Vault::create(&root, "synthetic-passphrase").unwrap();
        vault.set_automatic_evaluation(false).unwrap();
        vault.save_note(None, "Synthetic", "Preserved").unwrap();
        vault.db.execute_batch("DROP TABLE import_progress; DROP TABLE data_recent; DROP TABLE document_folder; DROP TABLE credential_record; DROP TABLE credential_import; DROP TABLE ai_question; DROP TABLE extraction_checkpoint; ALTER TABLE document_evaluation DROP COLUMN warning_message; PRAGMA user_version=3;").unwrap();
        drop(vault);
        let vault = Vault::unlock(&root, "synthetic-passphrase").unwrap();
        assert!(!vault.settings().unwrap().automatic_evaluation);
        assert_eq!(vault.collection("Preserved", false).unwrap().total, 1);
        assert_eq!(
            vault
                .db
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            9
        );
    }

    #[test]
    fn schema_four_migration_preserves_imports_and_explains_legacy_missing_candidates() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        let mut vault = Vault::create(&root, "synthetic-passphrase").unwrap();
        let file = temp.path().join("test.txt");
        fs::write(&file, "SYNTHETIC original").unwrap();
        let item = vault
            .import_document(&file, "test", DocumentClass::Personal)
            .unwrap();
        vault.set_automatic_evaluation(false).unwrap();
        vault.db.execute("UPDATE document_evaluation SET state='done',warning_message='6 unbelegte Angaben ausgelassen.'", []).unwrap();
        vault
            .db
            .execute_batch("DROP TABLE import_progress; DROP TABLE data_recent; DROP TABLE document_folder; DROP TABLE credential_record; DROP TABLE credential_import; DROP TABLE ai_question; PRAGMA user_version=4;")
            .unwrap();
        drop(vault);
        let mut vault = Vault::unlock(&root, "synthetic-passphrase").unwrap();
        assert!(!vault.settings().unwrap().automatic_evaluation);
        assert_eq!(vault.collection("", false).unwrap().total, 1);
        assert!(vault.review_questions(item).unwrap().is_empty());
        assert!(
            vault
                .evaluation_warning(item)
                .unwrap()
                .unwrap()
                .contains("Analyze again")
        );
        assert!(vault.begin_evaluation(item, false).unwrap());
    }

    #[test]
    fn future_schema_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        let vault = Vault::create(&root, "synthetic-passphrase-2026").unwrap();
        vault.db.pragma_update(None, "user_version", 999).unwrap();
        drop(vault);
        assert!(matches!(
            Vault::unlock(&root, "synthetic-passphrase-2026"),
            Err(Error::Format)
        ));
    }
}
