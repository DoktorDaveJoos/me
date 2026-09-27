//! Transactional domain history. Local cursors are never conflict-resolution clocks.
//! Export is encrypted; no network transport or remote authorization is implied.
use crate::{Error, Result, Vault, crypto};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EncryptedRevision {
    pub id: String,
    pub ciphertext: Vec<u8>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RevisionBatch {
    pub local_cursor: i64,
    pub revisions: Vec<EncryptedRevision>,
}

struct Record {
    table: &'static str,
    key: String,
    payload: String,
    scope: &'static str,
}
fn records(db: &Connection) -> Result<Vec<Record>> {
    let ordinary = [
        ("entity", "id", "personal"),
        ("property_definition", "key", "personal"),
        ("source", "id", "personal"),
        ("inbox_item", "id", "personal"),
        ("extraction_run", "id", "personal"),
        ("assertion", "id", "personal"),
        ("review_case", "id", "personal"),
        ("ai_proposal", "id", "personal"),
        ("ai_question", "id", "personal"),
        ("entity_identifier", "id", "personal"),
        ("entity_merge", "id", "personal"),
        ("observation", "id", "personal"),
        ("observation_resolution", "id", "personal"),
        ("credential_identity", "id", "credential"),
        ("secret_version", "id", "credential"),
        ("principal", "id", "authority"),
        ("capability_grant", "id", "authority"),
        ("action_intent", "id", "authority"),
        ("action_revision", "id", "authority"),
        ("action_approval", "id", "authority"),
        ("action_attempt", "id", "authority"),
        ("access_audit", "id", "authority"),
        ("document_profile", "source_id", "personal"),
        ("assertion_review", "assertion_id", "personal"),
        ("household_member", "entity_id", "personal"),
    ];
    let mut all = Vec::new();
    for (table, key, scope) in ordinary {
        // A stepwise migration journals later tables once their own migration ran.
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
            [table],
            |r| r.get(0),
        )?;
        if !exists {
            continue;
        }
        let columns = db
            .prepare(&format!("PRAGMA table_info({table})"))?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        all.push(Record {
            table,
            key: format!("@.{key}"),
            payload: format!(
                "json_object({})",
                columns
                    .iter()
                    .map(|c| format!("'{c}',@.{c}"))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            scope,
        });
    }
    for (table, columns) in [
        (
            "entity_alias",
            vec!["entity_id", "normalized_alias", "alias"],
        ),
        (
            "source_entity",
            vec!["source_id", "entity_id", "role", "status"],
        ),
        (
            "source_link",
            vec!["parent_id", "child_id", "role", "locator_json"],
        ),
        ("inbox_source", vec!["inbox_id", "source_id"]),
        (
            "assertion_evidence",
            vec!["assertion_id", "source_id", "evidence_key", "locator_json"],
        ),
        (
            "assertion_derivation",
            vec!["assertion_id", "input_id", "rule_version"],
        ),
        ("review_member", vec!["review_id", "assertion_id"]),
    ] {
        let keys = match table {
            "entity_alias" | "assertion_derivation" => 2,
            "source_entity" | "source_link" | "assertion_evidence" => 3,
            _ => columns.len(),
        };
        all.push(Record {
            table,
            key: format!(
                "json_array({})",
                columns[..keys]
                    .iter()
                    .map(|c| format!("@.{c}"))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            payload: format!(
                "json_object({})",
                columns
                    .iter()
                    .map(|c| format!("'{c}',@.{c}"))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            scope: "personal",
        });
    }
    all.push(Record{table:"collection_item",key:"@.stable_id".into(),payload:"json_object('id',@.stable_id,'title',@.title,'kind',@.kind,'source_id',@.source_id,'property_key',@.property_key,'current_assertion_id',@.current_assertion_id,'extension',@.extension,'pinned',@.pinned,'revision',@.revision,'deleted_at',@.deleted_at)".into(),scope:"personal"});
    all.push(Record{table:"document_folder",key:"(SELECT stable_id FROM collection_item WHERE local_id=@.item_id)".into(),payload:"json_object('item_id',(SELECT stable_id FROM collection_item WHERE local_id=@.item_id),'path',@.path)".into(),scope:"personal"});
    // Original archive bytes are separate encrypted assets; the journal contains their identity.
    all.push(Record{table:"credential_import",key:"@.id".into(),payload:"json_object('id',@.id,'fingerprint',@.fingerprint,'imported_at',@.imported_at,'asset_kind','credential_archive')".into(),scope:"credential"});
    all.push(Record{table:"credential_record",key:"(SELECT stable_id FROM collection_item WHERE local_id=@.item_id)".into(),payload:"json_object('item_id',(SELECT stable_id FROM collection_item WHERE local_id=@.item_id),'import_id',@.import_id,'account_uuid',@.account_uuid,'vault_uuid',@.vault_uuid,'item_uuid',@.item_uuid,'fingerprint',@.fingerprint,'vault_name',@.vault_name,'category',@.category,'archived',@.archived,'raw_json',@.raw_json,'attachment_paths_json',@.attachment_paths_json,'format',@.format)".into(),scope:"credential"});
    // Preserve causal decision identity without exporting device-local row numbers.
    all.push(Record{table:"decision",key:"@.id".into(),payload:"json_object('id',@.id,'assertion_id',@.assertion_id,'action',@.action,'actor',@.actor,'policy_version',@.policy_version,'reason_code',@.reason_code,'recorded_at',@.recorded_at,'previous_decision_id',(SELECT id FROM decision WHERE assertion_id=@.assertion_id AND local_seq<@.local_seq ORDER BY local_seq DESC LIMIT 1))".into(),scope:"personal"});
    Ok(all)
}
fn change_sql(record: &Record, row: &str, delete: bool, where_clause: &str) -> String {
    let key = record.key.replace('@', row);
    let payload = if delete {
        "'{}'".into()
    } else {
        record.payload.replace('@', row)
    };
    let scope = match record.table {
        "source" => format!(
            "CASE WHEN {row}.sensitivity='credential' THEN 'credential' ELSE 'personal' END"
        ),
        "collection_item" => {
            format!("CASE WHEN {row}.kind='credential' THEN 'credential' ELSE 'personal' END")
        }
        _ => format!("'{}'", record.scope),
    };
    let actor = match record.table {
        "access_audit" => format!("{row}.principal_id"),
        "entity_merge" | "observation_resolution" | "action_approval" => format!("{row}.actor_id"),
        "action_attempt" => format!(
            "coalesce((SELECT principal_id FROM capability_grant WHERE id={row}.grant_id),'system:executor')"
        ),
        "observation" | "extraction_run" => "'system:extraction'".into(),
        _ => "(SELECT profile_id FROM vault_meta)".into(),
    };
    format!("INSERT INTO domain_revision(id,record_kind,record_id,parents_json,operation,payload_json,format_version,key_scope,actor_id,device_id,recorded_at)
 SELECT lower(hex(randomblob(16))),'{table}',{key},coalesce((SELECT json_array(revision_id) FROM domain_head WHERE record_kind='{table}' AND record_id={key}),'[]'),'{operation}',{payload},1,{scope},{actor},(SELECT device_id FROM vault_meta),strftime('%Y-%m-%dT%H:%M:%fZ','now') {where_clause};
 INSERT INTO domain_head SELECT record_kind,record_id,id FROM domain_revision WHERE sequence=last_insert_rowid() ON CONFLICT(record_kind,record_id) DO UPDATE SET revision_id=excluded.revision_id;",table=record.table,operation=if delete{"delete"}else{"put"})
}
pub(crate) fn initialize(db: &Connection, bootstrap: bool) -> Result<()> {
    let all = records(db)?;
    for record in &all {
        for (event, row, delete) in [
            ("INSERT", "NEW", false),
            ("UPDATE", "NEW", false),
            ("DELETE", "OLD", true),
        ] {
            // Immutable records have no updates; installing the trigger is still harmless.
            let sql = change_sql(record, row, delete, "");
            db.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS journal_{table}_{event} AFTER {event} ON {table} WHEN EXISTS(SELECT 1 FROM vault_meta) BEGIN {sql} END;",table=record.table))?;
        }
    }
    // Compatibility writes create immutable protected versions atomically.
    for event in ["INSERT", "UPDATE"] {
        db.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS credential_version_{event} AFTER {event} ON credential_record BEGIN
   INSERT OR IGNORE INTO credential_identity(id) SELECT stable_id FROM collection_item WHERE local_id=NEW.item_id;
   INSERT INTO secret_version SELECT lower(hex(randomblob(16))),i.stable_id,(SELECT version_id FROM secret_head WHERE credential_id=i.stable_id),NEW.format,NEW.raw_json,strftime('%Y-%m-%dT%H:%M:%fZ','now') FROM collection_item i WHERE i.local_id=NEW.item_id;
   INSERT INTO secret_head SELECT credential_id,id FROM secret_version WHERE rowid=last_insert_rowid() ON CONFLICT(credential_id) DO UPDATE SET version_id=excluded.version_id;
   END;"))?;
    }
    if bootstrap {
        // Bootstrap in key order for repeatable enumeration; IDs are opaque, never clocks.
        for record in &all {
            let ids = db
                .prepare(&format!(
                    "SELECT rowid FROM {} ORDER BY rowid",
                    record.table
                ))?
                .query_map([], |r| r.get::<_, i64>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            for rowid in ids {
                let key = record.key.replace('@', "r");
                let exists:bool=db.query_row(&format!("SELECT EXISTS(SELECT 1 FROM {table} r JOIN domain_head h ON h.record_kind='{table}' AND h.record_id={key} WHERE r.rowid=?)",table=record.table),[rowid],|r|r.get(0))?;
                if !exists {
                    db.execute_batch(&change_sql(
                        record,
                        "r",
                        false,
                        &format!("FROM {} r WHERE r.rowid={rowid}", record.table),
                    ))?;
                }
            }
        }
        db.execute_batch("INSERT OR IGNORE INTO credential_identity(id) SELECT i.stable_id FROM collection_item i JOIN credential_record c ON c.item_id=i.local_id;
  INSERT INTO secret_version SELECT lower(hex(randomblob(16))),i.stable_id,NULL,c.format,c.raw_json,strftime('%Y-%m-%dT%H:%M:%fZ','now') FROM collection_item i JOIN credential_record c ON c.item_id=i.local_id WHERE NOT EXISTS(SELECT 1 FROM secret_head h WHERE h.credential_id=i.stable_id);
  INSERT OR IGNORE INTO secret_head SELECT credential_id,id FROM secret_version;
  INSERT OR IGNORE INTO observation SELECT p.id,p.source_id,p.run_id,p.id,p.property_key,json_quote(p.value),p.subject_quote,'',json_object('segment_id',p.segment_id,'quote',p.quote,'source_locator',json((SELECT locator_json FROM source_segment WHERE id=p.segment_id))),'supported',strftime('%Y-%m-%dT%H:%M:%fZ','now') FROM ai_proposal p;
  INSERT OR IGNORE INTO observation SELECT q.id,q.source_id,q.run_id,q.id,q.property_key,json_quote(q.value),q.claimed_subject,'',json_object('segment_id',q.segment_id,'claimed_quote',q.claimed_quote),'unverified',strftime('%Y-%m-%dT%H:%M:%fZ','now') FROM ai_question q;")?;
    }
    Ok(())
}
impl Vault {
    /// Bounded encrypted export. The cursor belongs only to this database instance.
    pub fn encrypted_revisions(&self, after: i64, limit: usize) -> Result<RevisionBatch> {
        if after < 0 || !(1..=100).contains(&limit) {
            return Err(Error::Validation("Invalid revision cursor or batch limit."));
        }
        let mut stmt=self.db.prepare("SELECT sequence,id,json_object('id',id,'record_kind',record_kind,'record_id',record_id,'parents',json(parents_json),'operation',operation,'payload',json(payload_json),'format_version',format_version,'key_scope',key_scope,'actor_id',actor_id,'device_id',device_id,'recorded_at',recorded_at) FROM domain_revision WHERE sequence>? ORDER BY sequence LIMIT ?")?;
        let rows = stmt.query_map(params![after, limit as i64], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let mut batch = RevisionBatch {
            local_cursor: after,
            revisions: Vec::new(),
        };
        let mut bytes = 0;
        for row in rows {
            let (cursor, id, raw) = row?;
            let raw = Zeroizing::new(raw);
            if bytes + raw.len() > 8 * 1024 * 1024 && !batch.revisions.is_empty() {
                break;
            }
            let cached: Option<Vec<u8>> = self
                .db
                .query_row(
                    "SELECT ciphertext FROM replication_envelope WHERE revision_id=?",
                    [&id],
                    |r| r.get(0),
                )
                .optional()?;
            let ciphertext = if let Some(ciphertext) = cached {
                ciphertext
            } else {
                let ciphertext = crypto::seal(
                    &self.keys[32..],
                    raw.as_bytes(),
                    format!("me-domain-revision-v1:{}:{id}", self.header.vault_id).as_bytes(),
                )?;
                self.db.execute(
                    "INSERT INTO replication_envelope VALUES(?,?)",
                    params![id, ciphertext],
                )?;
                ciphertext
            };
            bytes += raw.len();
            batch.local_cursor = cursor;
            batch.revisions.push(EncryptedRevision { id, ciphertext });
        }
        Ok(batch)
    }
    pub fn encrypted_replication_asset(&self, kind: &str, asset_id: &str) -> Result<Vec<u8>> {
        let cached: Option<Vec<u8>> = self
            .db
            .query_row(
                "SELECT ciphertext FROM replication_asset_envelope WHERE kind=? AND asset_id=?",
                params![kind, asset_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(ciphertext) = cached {
            return Ok(ciphertext);
        }
        let bytes = match kind {
            "document" => self.read_object(asset_id)?,
            "credential_archive" => Zeroizing::new(self.db.query_row(
                "SELECT archive FROM credential_import WHERE id=?",
                [asset_id],
                |r| r.get::<_, Vec<u8>>(0),
            )?),
            _ => return Err(Error::Validation("Unknown replication asset kind.")),
        };
        let ciphertext = crypto::seal(
            &self.keys[32..],
            &bytes,
            format!(
                "me-domain-asset-v1:{}:{kind}:{asset_id}",
                self.header.vault_id
            )
            .as_bytes(),
        )?;
        self.db.execute(
            "INSERT INTO replication_asset_envelope VALUES(?,?,?)",
            params![kind, asset_id, ciphertext],
        )?;
        Ok(ciphertext)
    }
    pub fn link_credential(&mut self, credential: &str, entity: &str, service: &str) -> Result<()> {
        let entity = crate::domain::canonical_entity(&self.db, entity)?;
        let service = crate::domain::canonical_entity(&self.db, service)?;
        if self.db.execute("UPDATE credential_identity SET entity_id=?,service_id=? WHERE id=? AND EXISTS(SELECT 1 FROM collection_item WHERE stable_id=? AND deleted_at IS NULL)",params![entity,service,credential,credential])?!=1{return Err(Error::Validation("Credential unavailable."));}
        Ok(())
    }
    pub fn credential_version_ids(&self, credential: &str) -> Result<Vec<String>> {
        Ok(self
            .db
            .prepare("SELECT id FROM secret_version WHERE credential_id=? ORDER BY rowid")?
            .query_map([credential], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?)
    }
    pub fn device_identity(&self) -> Result<String> {
        Ok(self
            .db
            .query_row("SELECT device_id FROM vault_meta", [], |r| r.get(0))?)
    }
}
// Tests which reconstruct an older schema must also remove the later domain layer.
#[cfg(test)]
pub(crate) fn remove_for_legacy_fixture(db: &Connection) {
    let triggers=db.prepare("SELECT name FROM sqlite_master WHERE type='trigger' AND (name LIKE 'journal_%' OR name LIKE 'credential_version_%' OR name LIKE 'observation_from_%')").unwrap().query_map([],|r|r.get::<_,String>(0)).unwrap().collect::<std::result::Result<Vec<_>,_>>().unwrap();
    for t in triggers {
        db.execute_batch(&format!("DROP TRIGGER {t}")).unwrap();
    }
    db.execute_batch("PRAGMA foreign_keys=OFF; DROP INDEX IF EXISTS assertion_property_time; DROP INDEX IF EXISTS document_profile_family; ALTER TABLE document_evaluation DROP COLUMN priority;")
        .unwrap();
    for table in [
        "merge_proposal",
        "household_proposal",
        "person_mention",
        "identity_setup",
        "household_member",
        "review_outcome",
        "assertion_review",
        "document_profile",
        "stage_run",
        "envelope_source",
        "intake_envelope",
        "import_batch",
        "replication_envelope",
        "replication_asset_envelope",
        "domain_head",
        "domain_revision",
        "entity_identifier",
        "entity_merge",
        "observation_resolution",
        "observation",
        "assertion_derivation",
        "secret_head",
        "secret_version",
        "credential_identity",
        "access_audit",
        "action_attempt",
        "action_approval",
        "action_head",
        "action_revision",
        "action_intent",
        "capability_grant",
        "principal",
    ] {
        db.execute_batch(&format!("DROP TABLE {table}")).unwrap();
    }
    db.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
}
