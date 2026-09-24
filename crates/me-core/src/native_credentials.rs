//! Native credential templates and atomic edits. No plaintext indexing or AI export.
use crate::{
    credentials::{SecretJson, json},
    vault::{hash, id, sql_id},
    *,
};
use rusqlite::params;
use serde_json::{Value, json as value};
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialKind {
    Login,
    Password,
    Api,
    Ssh,
    Wifi,
    Server,
}
impl CredentialKind {
    pub const ALL: [Self; 6] = [
        Self::Login,
        Self::Password,
        Self::Api,
        Self::Ssh,
        Self::Wifi,
        Self::Server,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::Password => "password",
            Self::Api => "api",
            Self::Ssh => "ssh",
            Self::Wifi => "wifi",
            Self::Server => "server",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Login => "Login",
            Self::Password => "Password / secret",
            Self::Api => "API credential",
            Self::Ssh => "SSH key",
            Self::Wifi => "Wi-Fi",
            Self::Server => "Server / database",
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            Self::Login => "An account, password and recovery codes",
            Self::Password => "A standalone password, PIN or access code",
            Self::Api => "A token, provider, project and permissions",
            Self::Ssh => "An existing private/public key pair",
            Self::Wifi => "A network name and password",
            Self::Server => "A host, port and access credentials",
        }
    }
    pub fn category(self) -> &'static str {
        match self {
            Self::Login => "001",
            Self::Password => "005",
            Self::Api => "me-api",
            Self::Ssh => "114",
            Self::Wifi => "109",
            Self::Server => "110",
        }
    }
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.key() == key)
    }
}
// (key, label, concealed, multiline). Field semantics cannot be changed by a model.
fn template(kind: CredentialKind) -> Vec<(&'static str, &'static str, bool, bool)> {
    let mut fields = match kind {
        CredentialKind::Login => vec![
            ("username", "Username / email", false, false),
            ("password", "Password", true, false),
            ("website", "Website", false, false),
            ("full_name", "Name", false, false),
            ("totp", "Two-factor setup key", true, false),
            ("recovery_codes", "Recovery codes", true, true),
        ],
        CredentialKind::Password => vec![
            ("password", "Password / secret", true, true),
            ("website", "Website (optional)", false, false),
        ],
        CredentialKind::Api => vec![
            ("token", "API key / token", true, true),
            ("website", "Provider website", false, false),
            ("username", "Account / key ID", false, false),
            ("project", "Project", false, false),
            ("environment", "Environment", false, false),
            ("scopes", "Permissions / scopes", false, true),
            ("expires", "Expires (YYYY-MM-DD)", false, false),
        ],
        CredentialKind::Ssh => vec![
            ("private_key", "Private key", true, true),
            ("public_key", "Public key", false, true),
            ("passphrase", "Passphrase", true, false),
            ("hostname", "Host / service", false, false),
            ("username", "Username", false, false),
        ],
        CredentialKind::Wifi => vec![
            ("ssid", "Network name (SSID)", false, false),
            ("password", "Password", true, false),
            ("security", "Security type", false, false),
        ],
        CredentialKind::Server => vec![
            ("hostname", "Host", false, false),
            ("port", "Port", false, false),
            ("username", "Username", false, false),
            ("password", "Password", true, false),
            ("database", "Database / service", false, false),
        ],
    };
    fields.extend([
        ("tags", "Tags", false, true),
        ("notes", "Notes", true, true),
    ]);
    if kind == CredentialKind::Login {
        fields.sort_by_key(|(key, _, _, _)| {
            [
                "username",
                "password",
                "website",
                "full_name",
                "tags",
                "notes",
                "totp",
                "recovery_codes",
            ]
            .iter()
            .position(|k| k == key)
            .unwrap_or(usize::MAX)
        });
    }
    fields
}
pub fn credential_draft(kind: CredentialKind) -> LoginDetails {
    let fields = template(kind)
        .into_iter()
        .map(|(key, label, concealed, multiline)| LoginField {
            key: key.into(),
            label: label.into(),
            section: if key == "notes" || key == "tags" {
                "Details"
            } else if key == "totp" || key == "recovery_codes" {
                "Recovery & two-factor"
            } else {
                kind.label()
            }
            .into(),
            value: Zeroizing::new(String::new()),
            concealed,
            multiline,
            kind: if key == "tags" {
                LoginValueKind::Tags
            } else {
                LoginValueKind::Text
            },
            editable: true,
            presentation: if key == "tags" {
                LoginPresentation::Tags
            } else {
                LoginPresentation::Value
            },
            used_until: None,
            used_until_date: None,
        })
        .collect();
    LoginDetails {
        credential: CredentialDetails {
            native: true,
            item_id: 0,
            title: String::new(),
            vault: "ME.".into(),
            category: kind.category().into(),
            archived: false,
            imported_at: String::new(),
            fields: Vec::new(),
            attachments: Vec::new(),
        },
        revision: 0,
        favorite: false,
        fields,
        native_kind: Some(kind),
    }
}
fn apply(
    data: &mut SecretJson,
    kind: CredentialKind,
    update: &LoginUpdate,
    now: i64,
) -> Result<()> {
    let fields = template(kind);
    let mut seen = std::collections::BTreeSet::new();
    for (key, text) in &update.fields {
        if !seen.insert(key) || !fields.iter().any(|(k, _, _, _)| k == key) {
            return Err(Error::Validation("This credential field cannot be edited."));
        }
        if text.len() > 1_048_576 {
            return Err(Error::Validation("A credential field exceeds 1 MB."));
        }
        if key == "port"
            && !text.is_empty()
            && text.parse::<u16>().ok().filter(|n| *n > 0).is_none()
        {
            return Err(Error::Validation("Enter a port between 1 and 65535."));
        }
        if key == "expires" && !text.is_empty() {
            let b = text.as_bytes();
            if b.len() != 10
                || b[4] != b'-'
                || b[7] != b'-'
                || !b
                    .iter()
                    .enumerate()
                    .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
            {
                return Err(Error::Validation("Enter an expiry date as YYYY-MM-DD."));
            }
        }
        let old = data.0["fields"][key].as_str().unwrap_or("");
        if matches!(
            key.as_str(),
            "password" | "token" | "private_key" | "passphrase" | "recovery_codes"
        ) && !old.is_empty()
            && old != text.as_str()
        {
            let entry = value!({"field":key,"value":old,"time":now});
            data.0["history"]
                .as_array_mut()
                .ok_or(Error::Format)?
                .push(entry);
        }
        data.0["fields"][key] = Value::String(text.to_string());
    }
    Ok(())
}
fn validate_title(title: &str) -> Result<&str> {
    let title = title.trim();
    if title.is_empty() || title.len() > 16_384 {
        return Err(Error::Validation(
            "Enter a title between 1 and 16,384 bytes.",
        ));
    }
    Ok(title)
}
impl Vault {
    pub(crate) fn native_kind(&self, item: u64) -> Result<Option<CredentialKind>> {
        use rusqlite::OptionalExtension;
        let kind:Option<String>=self.db.query_row("SELECT json_extract(c.raw_json,'$.kind') FROM credential_record c JOIN collection_item i ON i.local_id=c.item_id WHERE c.item_id=? AND c.format='me-v1' AND i.deleted_at IS NULL",[sql_id(item)?],|r|r.get(0)).optional()?;
        kind.map(|k| CredentialKind::from_key(&k).ok_or(Error::Format))
            .transpose()
    }
    pub fn create_credential(&mut self, kind: CredentialKind, update: LoginUpdate) -> Result<u64> {
        let title = validate_title(&update.title)?;
        let tx = self.db.transaction()?;
        let now = tx.query_row("SELECT unixepoch()", [], |r| r.get(0))?;
        let mut data = SecretJson(value!({"kind":kind.key(),"fields":{},"history":[]}));
        apply(&mut data, kind, &update, now)?;
        let raw = Zeroizing::new(serde_json::to_string(&data.0).map_err(|_| Error::Format)?);
        let stable = id();
        let source = id();
        tx.execute("INSERT INTO source(id,kind,title,received_at,content_fingerprint,sensitivity,retention) VALUES(?,'form',?,strftime('%Y-%m-%dT%H:%M:%fZ','now'),?,'credential','keep')",params![source,title,hash(raw.as_bytes())])?;
        tx.execute("INSERT INTO collection_item(stable_id,title,kind,source_id,pinned) VALUES(?,?,'credential',?,?)",params![stable,title,source,update.favorite])?;
        let item = tx.last_insert_rowid();
        tx.execute("INSERT INTO credential_record(item_id,import_id,account_uuid,vault_uuid,item_uuid,fingerprint,vault_name,category,archived,raw_json,attachment_paths_json,format) VALUES(?,NULL,'me',?,?,?,'ME.',?,0,?,'[]','me-v1')",params![item,self.header.vault_id,stable,hash(raw.as_bytes()),kind.category(),raw.as_str()])?;
        tx.execute("INSERT INTO local_change(operation_id,item_id,revision,kind,payload_json,recorded_at) VALUES(?,?,1,'credential_create','{}',strftime('%Y-%m-%dT%H:%M:%fZ','now'))",params![id(),stable])?;
        tx.commit()?;
        Ok(item as u64)
    }
    pub(crate) fn native_details(&self, item: u64, kind: CredentialKind) -> Result<LoginDetails> {
        let (title,raw,revision,favorite,date,archived):(String,String,i64,bool,String,bool)=self.db.query_row("SELECT i.title,c.raw_json,i.revision,i.pinned,s.received_at,c.archived FROM credential_record c JOIN collection_item i ON i.local_id=c.item_id JOIN source s ON s.id=i.source_id WHERE c.item_id=? AND i.deleted_at IS NULL",[sql_id(item)?],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
        let raw = Zeroizing::new(raw);
        let data = json(raw.as_bytes())?;
        let mut details = credential_draft(kind);
        details.credential.item_id = item;
        details.credential.title = title;
        details.credential.imported_at = date;
        details.credential.archived = archived;
        details.revision = revision;
        details.favorite = favorite;
        for f in &mut details.fields {
            f.value = Zeroizing::new(data.0["fields"][&f.key].as_str().unwrap_or("").into());
        }
        for (i, h) in data.0["history"]
            .as_array()
            .ok_or(Error::Format)?
            .iter()
            .enumerate()
            .rev()
        {
            let key = h["field"].as_str().ok_or(Error::Format)?;
            let label = template(kind)
                .into_iter()
                .find(|f| f.0 == key)
                .ok_or(Error::Format)?
                .1;
            let time = h["time"].as_i64().ok_or(Error::Format)?;
            details.fields.push(LoginField {
                key: format!("history/{i}"),
                label: format!("Previous {label}"),
                section: "History".into(),
                value: Zeroizing::new(h["value"].as_str().ok_or(Error::Format)?.into()),
                concealed: true,
                kind: LoginValueKind::Text,
                editable: false,
                multiline: true,
                presentation: LoginPresentation::History,
                used_until: Some(time),
                used_until_date: self.db.query_row(
                    "SELECT strftime('%Y-%m-%d %H:%M UTC',?,'unixepoch')",
                    [time],
                    |r| r.get(0),
                )?,
            });
        }
        details.credential.fields = details
            .fields
            .iter()
            .filter(|f| !f.value.is_empty())
            .map(|f| CredentialField {
                label: f.label.clone(),
                value: f.value.clone(),
            })
            .collect();
        Ok(details)
    }
    pub(crate) fn update_native(
        &mut self,
        item: u64,
        kind: CredentialKind,
        update: LoginUpdate,
    ) -> Result<()> {
        let title = validate_title(&update.title)?;
        let tx = self.db.transaction()?;
        let (raw,revision,stable,source):(String,i64,String,String)=tx.query_row("SELECT c.raw_json,i.revision,i.stable_id,i.source_id FROM credential_record c JOIN collection_item i ON i.local_id=c.item_id WHERE c.item_id=? AND i.deleted_at IS NULL",[sql_id(item)?],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
        if revision != update.revision {
            return Err(Error::Validation(
                "This item changed. Cancel editing and reopen it before saving.",
            ));
        }
        let raw = Zeroizing::new(raw);
        let mut data = json(raw.as_bytes())?;
        apply(
            &mut data,
            kind,
            &update,
            tx.query_row("SELECT unixepoch()", [], |r| r.get(0))?,
        )?;
        let raw = Zeroizing::new(serde_json::to_string(&data.0).map_err(|_| Error::Format)?);
        tx.execute(
            "UPDATE credential_record SET raw_json=? WHERE item_id=?",
            params![raw.as_str(), sql_id(item)?],
        )?;
        tx.execute(
            "UPDATE collection_item SET title=?,pinned=?,revision=revision+1 WHERE local_id=?",
            params![title, update.favorite, sql_id(item)?],
        )?;
        tx.execute(
            "UPDATE source SET title=? WHERE id=?",
            params![title, source],
        )?;
        tx.execute("INSERT INTO local_change(operation_id,item_id,revision,kind,payload_json,recorded_at) VALUES(?,?,?,'credential_edit','{}',strftime('%Y-%m-%dT%H:%M:%fZ','now'))",params![id(),stable,revision+1])?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn update(revision: i64, fields: &[(&str, &str)]) -> LoginUpdate {
        LoginUpdate {
            revision,
            title: "Synthetic credential".into(),
            favorite: false,
            fields: fields
                .iter()
                .map(|(k, v)| (k.to_string(), Zeroizing::new(v.to_string())))
                .collect(),
        }
    }
    #[test]
    fn v12_migration_preserves_imports_and_allows_native_creation() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("vault");
        let mut v = Vault::create(&root, "synthetic-passphrase").unwrap();
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/synthetic-logins.1pux");
        v.import_onepassword(&OnePasswordImport::read(&fixture).unwrap())
            .unwrap();
        let item = v.logins().unwrap()[0].id;
        let before = v.login_details(item).unwrap().fields[1].value.clone();
        let archive: Vec<u8> =
            v.db.query_row("SELECT archive FROM credential_import LIMIT 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        v.db.execute_batch("ALTER TABLE credential_record RENAME TO credential_record_new;")
            .unwrap();
        let schema = include_str!("../migrations/006_credentials.sql")
            .split("CREATE TABLE credential_record")
            .nth(1)
            .unwrap();
        v.db.execute_batch(&format!("CREATE TABLE credential_record{schema}"))
            .unwrap();
        v.db.execute_batch("INSERT INTO credential_record SELECT item_id,import_id,account_uuid,vault_uuid,item_uuid,fingerprint,vault_name,category,archived,raw_json,attachment_paths_json FROM credential_record_new; DROP TABLE credential_record_new; PRAGMA user_version=12;").unwrap();
        drop(v);
        let mut v = Vault::unlock(&root, "synthetic-passphrase").unwrap();
        assert_eq!(
            v.login_details(item).unwrap().fields[1].value.as_str(),
            before.as_str()
        );
        let after: Vec<u8> =
            v.db.query_row("SELECT archive FROM credential_import LIMIT 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(archive, after);
        v.update_login(
            item,
            update(1, &[("$recovery_codes", "ABCD-EFGH\nIJKL-MNOP")]),
        )
        .unwrap();
        assert!(
            v.login_details(item)
                .unwrap()
                .fields
                .iter()
                .any(|f| f.value.as_str() == "ABCD-EFGH\nIJKL-MNOP")
        );
        v.create_credential(CredentialKind::Api, update(0, &[("token", "SYNTHETIC")]))
            .unwrap();
        assert_eq!(v.logins().unwrap().len(), 11);
    }
    #[test]
    fn generated_passwords_use_the_requested_alphabet_and_are_not_repeated() {
        let a = generate_credential_password().unwrap();
        let b = generate_credential_password().unwrap();
        assert_eq!(a.len(), 20);
        assert!(a.bytes().any(|b| b.is_ascii_punctuation()));
        assert_ne!(a.as_str(), b.as_str());
    }
    #[test]
    fn native_types_roundtrip_and_stay_out_of_fts_and_imports() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("vault");
        let mut v = Vault::create(&root, "synthetic-passphrase").unwrap();
        for kind in CredentialKind::ALL {
            let field = template(kind)[0].0;
            let item = v
                .create_credential(kind, update(0, &[(field, "SYNTHETIC-PRIVATE")]))
                .unwrap();
            let d = v.login_details(item).unwrap();
            assert_eq!(d.native_kind, Some(kind));
            assert!(
                d.fields
                    .iter()
                    .any(|f| f.value.as_str() == "SYNTHETIC-PRIVATE")
            );
            assert_eq!(v.credential_details(item).unwrap().item_id, item);
        }
        assert_eq!(v.logins().unwrap().len(), 6);
        assert_eq!(v.collection("", false).unwrap().items.len(), 6);
        assert!(
            v.collection("SYNTHETIC-PRIVATE", false)
                .unwrap()
                .items
                .is_empty()
        );
        for table in ["credential_import", "assertion", "source_segment", "job"] {
            let n: i64 =
                v.db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                    .unwrap();
            assert_eq!(n, 0, "{table}");
        }
        drop(v);
        let v = Vault::unlock(&root, "synthetic-passphrase").unwrap();
        assert_eq!(v.logins().unwrap().len(), 6);
    }
    #[test]
    fn native_updates_preserve_secrets_history_and_reject_stale_or_unknown_fields_atomically() {
        let tmp = tempfile::tempdir().unwrap();
        let mut v = Vault::create(&tmp.path().join("v"), "synthetic-passphrase").unwrap();
        let item = v
            .create_credential(
                CredentialKind::Login,
                update(0, &[("password", "  OLD\nsecret  ")]),
            )
            .unwrap();
        v.update_login(item, update(1, &[("password", "NEWsecret")]))
            .unwrap();
        let d = v.login_details(item).unwrap();
        assert_eq!(d.revision, 2);
        let h = d.fields.iter().find(|f| !f.editable).unwrap();
        assert_eq!(h.value.as_str(), "  OLD\nsecret  ");
        assert!(h.used_until_date.is_some());
        assert!(
            v.update_login(item, update(1, &[("password", "BAD")]))
                .is_err()
        );
        assert!(
            v.update_login(item, update(2, &[("password", "BAD"), ("unknown", "BAD")]))
                .is_err()
        );
        assert_eq!(v.login_details(item).unwrap().revision, 2);
        assert!(
            v.create_credential(CredentialKind::Api, update(0, &[("expires", "bad")]))
                .is_err()
        );
        assert_eq!(v.logins().unwrap().len(), 1);
    }
}
