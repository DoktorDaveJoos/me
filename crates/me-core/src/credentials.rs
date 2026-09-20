//! Local 1PUX import. Credentials never become assertions, document jobs or FTS text.
use crate::{
    Error, Result, Vault, crypto,
    vault::{hash, id, sql_id},
};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Read},
    path::Path,
};
use zeroize::{Zeroize, Zeroizing};
use zip::ZipArchive;

const MAX_ARCHIVE: u64 = 256 * 1024 * 1024;
const MAX_EXPANDED: u64 = 512 * 1024 * 1024;
const MAX_ENTRY: u64 = 128 * 1024 * 1024;
const MAX_DATA: u64 = 32 * 1024 * 1024;
const MAX_ITEMS: usize = 20_000;
const INVALID: &str = "This 1Password file is incomplete or damaged. Export it again as 1PUX.";
const TOO_LARGE: &str =
    "The export exceeds 256 MB, 512 MB unpacked, or 20,000 entries. Export smaller vaults.";

// No Debug/Serialize on secret-bearing application types.
pub struct OnePasswordImport {
    archive: Zeroizing<Vec<u8>>,
    fingerprint: String,
    items: Vec<ImportItem>,
    files: usize,
}
struct ImportItem {
    account: String,
    vault: String,
    uuid: String,
    vault_name: String,
    title: String,
    category: String,
    archived: bool,
    favorite: bool,
    raw: Zeroizing<String>,
    fingerprint: String,
    attachments: Vec<String>,
}
#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct OnePasswordSummary {
    pub total: usize,
    pub new: usize,
    pub duplicates: usize,
    pub changed: usize,
    pub archived: usize,
    pub files: usize,
    pub vaults: Vec<(String, usize)>,
}
impl OnePasswordSummary {
    pub fn to_import(&self) -> usize {
        self.new + self.changed
    }
}

pub struct CredentialField {
    pub label: String,
    pub value: Zeroizing<String>,
}
pub struct CredentialAttachment {
    pub name: String,
    pub(crate) path: String,
}
pub struct CredentialDetails {
    pub item_id: u64,
    pub title: String,
    pub vault: String,
    pub category: String,
    pub archived: bool,
    pub imported_at: String,
    pub fields: Vec<CredentialField>,
    pub attachments: Vec<CredentialAttachment>,
}

// serde_json does not wipe strings when Values are dropped. Wipe parsed values,
// including unknown fields; allocator/parser internals still have no such guarantee.
struct SecretJson(Value);
impl Drop for SecretJson {
    fn drop(&mut self) {
        wipe_json(&mut self.0);
    }
}
fn wipe_json(value: &mut Value) {
    match value {
        Value::String(s) => s.zeroize(),
        Value::Array(a) => a.iter_mut().for_each(wipe_json),
        Value::Object(o) => {
            for (mut key, mut value) in std::mem::take(o) {
                key.zeroize();
                wipe_json(&mut value);
            }
        }
        _ => {}
    }
}
fn invalid() -> Error {
    Error::Validation(INVALID)
}
fn json(bytes: &[u8]) -> Result<SecretJson> {
    serde_json::from_slice(bytes)
        .map(SecretJson)
        .map_err(|_| invalid())
}
fn array<'a>(v: &'a Value, key: &str) -> Result<&'a Vec<Value>> {
    v.get(key).and_then(Value::as_array).ok_or_else(invalid)
}
fn required<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key).and_then(Value::as_str).ok_or_else(invalid)
}
fn identifier<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    let s = required(v, key)?;
    if s.is_empty()
        || s.len() > 256
        || !s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(invalid());
    }
    Ok(s)
}
fn optional_array<'a>(v: &'a Value, key: &str) -> Result<&'a [Value]> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(&[]),
        Some(Value::Array(a)) => Ok(a),
        _ => Err(invalid()),
    }
}
fn optional_text<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(""),
        Some(Value::String(s)) => Ok(s),
        _ => Err(invalid()),
    }
}
fn bounded_entry(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    name: &str,
    limit: u64,
) -> Result<Zeroizing<Vec<u8>>> {
    let file = archive.by_name(name).map_err(|_| invalid())?;
    if file.size() > limit {
        return Err(Error::Validation(TOO_LARGE));
    }
    let mut data = Zeroizing::new(Vec::new());
    file.take(limit + 1)
        .read_to_end(&mut data)
        .map_err(|_| invalid())?;
    if data.len() as u64 > limit {
        return Err(Error::Validation(TOO_LARGE));
    }
    Ok(data)
}
fn safe_archive_name(name: &str) -> bool {
    !name.starts_with('/')
        && !name.contains(['\\', '\0'])
        && name
            .trim_end_matches('/')
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != "..")
}

impl OnePasswordImport {
    /// Read and validate off the UI thread. Never unpack to disk or contact a service.
    pub fn read(path: &Path) -> Result<Self> {
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("1pux"))
        {
            return Err(Error::Validation(
                "Choose a .1pux export. CSV and .1pif aren't supported.",
            ));
        }
        Self::from_bytes(crypto::read_bounded(path, MAX_ARCHIVE)?)
    }
    fn from_bytes(bytes: Zeroizing<Vec<u8>>) -> Result<Self> {
        let mut archive = ZipArchive::new(Cursor::new(bytes.as_slice())).map_err(|_| invalid())?;
        if archive.len() > 50_000 {
            return Err(Error::Validation(TOO_LARGE));
        }
        let mut names = BTreeSet::new();
        let mut files = BTreeMap::new();
        let mut expanded = 0u64;
        for index in 0..archive.len() {
            let file = archive.by_index(index).map_err(|_| invalid())?;
            let name = file.name().to_owned();
            if !safe_archive_name(&name)
                || !names.insert(name.clone())
                || file.encrypted()
                || file.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
            {
                return Err(invalid());
            }
            expanded = expanded.checked_add(file.size()).ok_or_else(invalid)?;
            if expanded > MAX_EXPANDED || file.size() > MAX_ENTRY {
                return Err(Error::Validation(TOO_LARGE));
            }
            let directory = file.is_dir();
            drop(file);
            if !directory {
                let limit = match name.as_str() {
                    "export.attributes" => 4096,
                    "export.data" => MAX_DATA,
                    _ => MAX_ENTRY,
                };
                let data = bounded_entry(&mut archive, &name, limit)?;
                if name.starts_with("files/") {
                    files.insert(name, hash(&data));
                }
            }
        }
        let attributes = json(&bounded_entry(&mut archive, "export.attributes", 4096)?)?;
        if attributes.0["version"].as_u64() != Some(3) {
            return Err(Error::Validation(
                "Use a version 3 export from 1Password 8.",
            ));
        }
        let data = json(&bounded_entry(&mut archive, "export.data", MAX_DATA)?)?;
        let mut items = Vec::new();
        let mut identities = BTreeSet::new();
        for account in array(&data.0, "accounts")? {
            let account_id = identifier(&account["attrs"], "uuid")?;
            for vault in array(account, "vaults")? {
                let vault_id = identifier(&vault["attrs"], "uuid")?;
                let vault_name = required(&vault["attrs"], "name")?;
                for item in array(vault, "items")? {
                    if items.len() == MAX_ITEMS {
                        return Err(Error::Validation(TOO_LARGE));
                    }
                    let uuid = identifier(item, "uuid")?;
                    if !identities.insert((account_id, vault_id, uuid)) {
                        return Err(invalid());
                    }
                    let overview = &item["overview"];
                    let title = required(overview, "title")?;
                    if title.len() > 16_384
                        || vault_name.len() > 16_384
                        || !item["details"].is_object()
                    {
                        return Err(invalid());
                    }
                    let category = required(item, "categoryUuid")?;
                    let archived = match required(item, "state")? {
                        "active" => false,
                        "archived" => true,
                        _ => return Err(invalid()),
                    };
                    let favorite = item["favIndex"].as_i64().ok_or_else(invalid)? > 0;
                    // Validate known display fields now, so a saved item can always be opened.
                    let _ = fields(item)?;
                    let attachments = attachment_paths(item, &files)?;
                    let raw = Zeroizing::new(serde_json::to_string(item).map_err(|_| invalid())?);
                    let mut fingerprint_input = Zeroizing::new(raw.to_string());
                    for path in &attachments {
                        fingerprint_input.push_str(path);
                        fingerprint_input.push_str(&files[path]);
                    }
                    items.push(ImportItem {
                        account: account_id.into(),
                        vault: vault_id.into(),
                        uuid: uuid.into(),
                        vault_name: vault_name.into(),
                        title: if title.trim().is_empty() {
                            "Untitled".into()
                        } else {
                            title.into()
                        },
                        category: category.into(),
                        archived,
                        favorite,
                        fingerprint: hash(fingerprint_input.as_bytes()),
                        raw,
                        attachments,
                    });
                }
            }
        }
        drop(archive);
        Ok(Self {
            fingerprint: hash(&bytes),
            archive: bytes,
            items,
            files: files.len(),
        })
    }
}

fn attachment_paths(item: &Value, files: &BTreeMap<String, String>) -> Result<Vec<String>> {
    fn visit(
        v: &Value,
        files: &BTreeMap<String, String>,
        found: &mut BTreeSet<String>,
    ) -> Result<()> {
        match v {
            Value::Object(o) => {
                for (key, value) in o {
                    if key == "documentId" || key == "documentID" {
                        let id = value.as_str().ok_or_else(invalid)?;
                        if id.is_empty() {
                            continue;
                        }
                        // Current exports use two underscores; the published format
                        // also documents three. Keep the delimiter so doc1 cannot
                        // accidentally resolve to a file belonging to doc10.
                        let prefix = format!("files/{id}__");
                        let exact = format!("files/{id}");
                        let matches: Vec<_> = files
                            .keys()
                            .filter(|p| *p == &exact || p.starts_with(&prefix))
                            .collect();
                        if matches.len() != 1 {
                            return Err(Error::Validation(
                                "An attachment is missing or ambiguous. Export again from 1Password.",
                            ));
                        }
                        found.insert(matches[0].clone());
                    }
                    if key == "path"
                        && let Some(path) = value.as_str().filter(|s| s.starts_with("files/"))
                    {
                        if !files.contains_key(path) {
                            return Err(invalid());
                        }
                        found.insert(path.to_owned());
                    }
                    visit(value, files, found)?;
                }
            }
            Value::Array(a) => {
                for v in a {
                    visit(v, files, found)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut found = BTreeSet::new();
    visit(item, files, &mut found)?;
    Ok(found.into_iter().collect())
}

// Prefer the original filename metadata: with both delimiters supported, a
// filename beginning with an underscore cannot be inferred from the ZIP name.
fn attachment_filename<'a>(item: &'a Value, path: &str) -> Option<&'a str> {
    match item {
        Value::Object(object) => {
            if let Some(id) = object
                .get("documentId")
                .or_else(|| object.get("documentID"))
                .and_then(Value::as_str)
                && let Some(name) = object.get("fileName").and_then(Value::as_str)
                && let Some(suffix) = path.strip_prefix("files/").and_then(|p| p.strip_prefix(id))
                && ["__", "___"]
                    .iter()
                    .any(|separator| suffix.strip_prefix(separator) == Some(name))
            {
                return Some(name);
            }
            object
                .values()
                .find_map(|value| attachment_filename(value, path))
        }
        Value::Array(array) => array
            .iter()
            .find_map(|value| attachment_filename(value, path)),
        _ => None,
    }
}

fn fields(item: &Value) -> Result<Vec<CredentialField>> {
    let mut result = Vec::new();
    let mut push = |label: String, value: &str| {
        if !value.is_empty() {
            result.push(CredentialField {
                label,
                value: Zeroizing::new(value.to_owned()),
            });
        }
    };
    let details = &item["details"];
    for field in optional_array(details, "loginFields")? {
        let designation = optional_text(field, "designation")?;
        let label = match designation {
            "username" => "Username",
            "password" => "Password",
            _ => optional_text(field, "name")?,
        };
        push(
            if label.is_empty() {
                "Form field".into()
            } else {
                label.into()
            },
            required(field, "value")?,
        );
    }
    push("Password".into(), optional_text(details, "password")?);
    let overview = &item["overview"];
    let primary = optional_text(overview, "url")?;
    push("Website".into(), primary);
    for url in optional_array(overview, "urls")? {
        let value = required(url, "url")?;
        if value != primary {
            push("Other website".into(), value);
        }
    }
    push("Notes".into(), optional_text(details, "notesPlain")?);
    for section in optional_array(details, "sections")? {
        let heading = optional_text(section, "title")?;
        for field in array(section, "fields")? {
            let title = optional_text(field, "title")?;
            let label = if heading.is_empty() {
                title.to_string()
            } else {
                format!("{heading} · {title}")
            };
            let value = field.get("value").ok_or_else(invalid)?;
            let display = if let Some(o) = value.as_object().filter(|o| o.len() == 1) {
                o.values().next().unwrap_or(value)
            } else {
                value
            };
            let text = Zeroizing::new(match display {
                Value::String(s) => s.clone(),
                _ => serde_json::to_string(display).map_err(|_| invalid())?,
            });
            push(
                if label.is_empty() {
                    "Custom field".into()
                } else {
                    label
                },
                &text,
            );
        }
    }
    for entry in optional_array(details, "passwordHistory")? {
        let timestamp = entry["time"].as_i64().ok_or_else(invalid)?;
        push(
            format!("Previous password · {timestamp}"),
            required(entry, "value")?,
        );
    }
    let tags = optional_array(overview, "tags")?;
    let tags = Zeroizing::new(
        tags.iter()
            .map(|t| t.as_str().ok_or_else(invalid))
            .collect::<Result<Vec<_>>>()?
            .join(", "),
    );
    push("Tags".into(), &tags);
    Ok(result)
}

pub fn credential_category(category: &str) -> &str {
    match category {
        "001" => "Login",
        "002" => "Credit card",
        "003" => "Secure note",
        "004" => "Identity",
        "005" => "Password",
        "006" => "Document",
        "100" => "Software license",
        "101" => "Bank account",
        "102" => "Database",
        "103" => "Driver's license",
        "104" => "Outdoor license",
        "105" => "Membership",
        "106" => "Passport",
        "107" => "Rewards program",
        "108" => "Social insurance",
        "109" => "Wi-Fi",
        "110" => "Server",
        "111" => "Email account",
        "114" => "SSH key",
        _ => "1Password entry",
    }
}

impl Vault {
    pub fn preview_onepassword(&self, import: &OnePasswordImport) -> Result<OnePasswordSummary> {
        summarize(&self.db, import)
    }
    /// All records and the original archive commit together. Changed versions are
    /// retained separately; no existing secret is ever silently overwritten.
    pub fn import_onepassword(&mut self, import: &OnePasswordImport) -> Result<OnePasswordSummary> {
        let tx = self.db.transaction()?;
        let summary = summarize(&tx, import)?;
        if summary.to_import() == 0 {
            return Ok(summary);
        }
        let import_id = id();
        tx.execute("INSERT OR IGNORE INTO credential_import VALUES(?,?,?,strftime('%Y-%m-%dT%H:%M:%fZ','now'))", params![import_id,import.fingerprint,import.archive.as_slice()])?;
        let import_id: String = tx.query_row(
            "SELECT id FROM credential_import WHERE fingerprint=?",
            [&import.fingerprint],
            |r| r.get(0),
        )?;
        for item in &import.items {
            if identity_status(&tx, item)? == IdentityStatus::Duplicate {
                continue;
            }
            let source = id();
            let stable = id();
            tx.execute("INSERT INTO source(id,kind,title,received_at,content_fingerprint,sensitivity,retention) VALUES(?,'form',?,strftime('%Y-%m-%dT%H:%M:%fZ','now'),?,'credential','keep')", params![source,item.title,item.fingerprint])?;
            tx.execute("INSERT INTO collection_item(stable_id,title,kind,source_id,pinned) VALUES(?,?,'credential',?,?)", params![stable,item.title,source,item.favorite])?;
            let local_id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO credential_record VALUES(?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    local_id,
                    import_id,
                    item.account,
                    item.vault,
                    item.uuid,
                    item.fingerprint,
                    item.vault_name,
                    item.category,
                    item.archived,
                    item.raw.as_str(),
                    serde_json::to_string(&item.attachments).map_err(|_| invalid())?
                ],
            )?;
            tx.execute("INSERT INTO local_change(operation_id,item_id,revision,kind,payload_json,recorded_at) VALUES(?,?,1,'credential_import','{}',strftime('%Y-%m-%dT%H:%M:%fZ','now'))", params![id(),stable])?;
        }
        tx.commit()?;
        Ok(summary)
    }
    pub fn credential_details(&self, item: u64) -> Result<CredentialDetails> {
        let (title, vault, category, archived, raw, paths, imported_at): (String,String,String,bool,String,String,String) = self.db.query_row(
            "SELECT i.title,c.vault_name,c.category,c.archived,c.raw_json,c.attachment_paths_json,a.imported_at FROM credential_record c JOIN collection_item i ON i.local_id=c.item_id JOIN credential_import a ON a.id=c.import_id WHERE c.item_id=? AND i.deleted_at IS NULL AND i.kind='credential'",
            [sql_id(item)?], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)))?;
        let raw = Zeroizing::new(raw);
        let data = json(raw.as_bytes())?;
        let paths = Zeroizing::new(paths);
        let paths: Vec<String> = serde_json::from_str(&paths).map_err(|_| invalid())?;
        let attachments = paths
            .into_iter()
            .map(|path| {
                let name = attachment_filename(&data.0, &path)
                    .or_else(|| {
                        path.rsplit('/')
                            .next()?
                            .split_once("__")
                            .map(|(_, name)| name.strip_prefix('_').unwrap_or(name))
                    })
                    .unwrap_or("Attachment")
                    .to_string();
                CredentialAttachment { name, path }
            })
            .collect();
        Ok(CredentialDetails {
            item_id: item,
            title,
            vault,
            category,
            archived,
            imported_at,
            fields: fields(&data.0)?,
            attachments,
        })
    }
    /// Explicit local export. The caller must make plaintext destination semantics visible.
    pub fn export_credential_attachment(
        &self,
        item: u64,
        attachment: usize,
        destination: &Path,
    ) -> Result<()> {
        let details = self.credential_details(item)?;
        let attachment = details.attachments.get(attachment).ok_or_else(invalid)?;
        let bytes = self.credential_archive(item)?;
        let mut archive = ZipArchive::new(Cursor::new(bytes.as_slice())).map_err(|_| invalid())?;
        let data = bounded_entry(&mut archive, &attachment.path, MAX_ENTRY)?;
        crypto::atomic_write(destination, &data)
    }
    /// Recovery/export includes every original field, attachment, custom icon and item.
    pub fn export_credential_original(&self, item: u64, destination: &Path) -> Result<()> {
        crypto::atomic_write(destination, &self.credential_archive(item)?)
    }
    fn credential_archive(&self, item: u64) -> Result<Zeroizing<Vec<u8>>> {
        let data = self.db.query_row("SELECT a.archive FROM credential_import a JOIN credential_record c ON c.import_id=a.id JOIN collection_item i ON i.local_id=c.item_id WHERE c.item_id=? AND i.deleted_at IS NULL", [sql_id(item)?], |r| r.get::<_,Vec<u8>>(0))?;
        Ok(Zeroizing::new(data))
    }
}
#[derive(PartialEq, Eq)]
enum IdentityStatus {
    New,
    Duplicate,
    Changed,
}
fn identity_status(db: &rusqlite::Connection, item: &ImportItem) -> Result<IdentityStatus> {
    let equal: Option<bool> = db.query_row("SELECT max(fingerprint=?4) FROM credential_record WHERE account_uuid=?1 AND vault_uuid=?2 AND item_uuid=?3 HAVING count(*)>0", params![item.account,item.vault,item.uuid,item.fingerprint], |r| r.get(0)).optional()?;
    Ok(match equal {
        Some(true) => IdentityStatus::Duplicate,
        Some(false) => IdentityStatus::Changed,
        None => IdentityStatus::New,
    })
}
fn summarize(db: &rusqlite::Connection, import: &OnePasswordImport) -> Result<OnePasswordSummary> {
    let mut result = OnePasswordSummary {
        total: import.items.len(),
        files: import.files,
        ..Default::default()
    };
    let mut vaults = BTreeMap::new();
    for item in &import.items {
        match identity_status(db, item)? {
            IdentityStatus::New => result.new += 1,
            IdentityStatus::Duplicate => result.duplicates += 1,
            IdentityStatus::Changed => result.changed += 1,
        }
        result.archived += usize::from(item.archived);
        *vaults.entry(item.vault_name.clone()).or_insert(0) += 1;
    }
    result.vaults = vaults.into_iter().collect();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{fs, io::Write};
    use zip::{ZipWriter, write::SimpleFileOptions};
    const PASSWORD: &str = "synthetic-vault-password";
    const SECRET: &str = "  SYNTHETIC-PASSWORD-unique-86402 🗝  ";
    fn login() -> Value {
        json!({"uuid":"item1","favIndex":1,"state":"active","categoryUuid":"001","createdAt":10,"updatedAt":20,
            "overview":{"title":"Synthetic Mail","url":"https://example.test/","urls":[{"url":"https://example.test/"},{"url":"https://other.test/"}],"tags":["synthetic","work"]},
            "details":{"loginFields":[{"designation":"username","value":"tester@example.test"},{"designation":"password","value":SECRET}],"notesPlain":"SYNTHETIC private note",
                "sections":[{"title":"Security","fields":[{"title":"PIN","value":{"concealed":"001234"}},{"title":"One-time password","value":{"totp":"otpauth://totp/Example?secret=TESTSECRET"}},{"title":"Address","value":{"address":{"city":"Berlin","zip":"00123"}}}]}],
                "passwordHistory":[{"value":"SYNTHETIC-OLD","time":5}],"futureField":{"secret":"SYNTHETIC-UNKNOWN"}}})
    }
    fn data(items: Vec<Value>) -> Value {
        json!({"accounts":[{"attrs":{"uuid":"account1"},"vaults":[{"attrs":{"uuid":"vault1","name":"Personal"},"items":items}]}]})
    }
    fn archive(data: &Value, extras: &[(&str, &[u8])]) -> Zeroizing<Vec<u8>> {
        zip_files(
            &[
                ("export.attributes", br#"{"version":3}"#.as_slice()),
                ("export.data", &serde_json::to_vec(data).unwrap()),
            ]
            .into_iter()
            .chain(extras.iter().copied())
            .collect::<Vec<_>>(),
        )
    }
    fn zip_files(files: &[(&str, &[u8])]) -> Zeroizing<Vec<u8>> {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in files {
            zip.start_file(
                *name,
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
            zip.write_all(bytes).unwrap();
        }
        Zeroizing::new(zip.finish().unwrap().into_inner())
    }
    fn batch(items: Vec<Value>) -> OnePasswordImport {
        OnePasswordImport::from_bytes(archive(&data(items), &[])).unwrap()
    }
    fn vault(dir: &tempfile::TempDir) -> Vault {
        Vault::create(&dir.path().join("vault"), PASSWORD).unwrap()
    }
    fn no_plaintext(path: &Path) {
        for e in fs::read_dir(path).unwrap() {
            let path = e.unwrap().path();
            if path.is_dir() {
                no_plaintext(&path);
            } else {
                let bytes = fs::read(path).unwrap();
                for text in [
                    SECRET,
                    "SYNTHETIC private note",
                    "SYNTHETIC-UNKNOWN",
                    "SYNTHETIC-ATTACHMENT",
                    "Synthetic Mail",
                ] {
                    assert!(!bytes.windows(text.len()).any(|w| w == text.as_bytes()));
                }
            }
        }
    }
    #[test]
    fn imports_fields_favorites_archive_and_restores_encrypted_backup() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = vault(&dir);
        let mut item = login();
        item["state"] = json!("archived");
        item["details"]["documentAttributes"] = json!({"documentId":"doc1","fileName":"key.txt"});
        let original = archive(
            &data(vec![item]),
            &[
                ("files/doc1___key.txt", b"SYNTHETIC-ATTACHMENT"),
                ("files/custom-icon.png", b"synthetic icon"),
            ],
        );
        let import = OnePasswordImport::from_bytes(original.clone()).unwrap();
        let preview = v.preview_onepassword(&import).unwrap();
        assert_eq!(
            (preview.total, preview.new, preview.archived, preview.files),
            (1, 1, 1, 2)
        );
        assert!(v.collection("", false).unwrap().is_empty()); // preview has no side effects
        assert_eq!(v.import_onepassword(&import).unwrap(), preview);
        let collection = v.collection("Synthetic", true).unwrap();
        let item = &collection.items[0];
        assert!(matches!(
            &item.content,
            crate::Content::Credential { archived: true, .. }
        ));
        let item_id = item.id;
        let details = v.credential_details(item_id).unwrap();
        assert!(
            details
                .fields
                .iter()
                .any(|f| f.label == "Password" && *f.value == SECRET)
        );
        assert!(details.fields.iter().any(|f| *f.value == "001234"));
        assert!(details.fields.iter().any(|f| *f.value == "SYNTHETIC-OLD"));
        assert!(
            details
                .fields
                .iter()
                .any(|f| f.value.contains("otpauth://"))
        );
        assert_eq!(details.attachments.len(), 1);
        let attachment = dir.path().join("attachment.txt");
        v.export_credential_attachment(item_id, 0, &attachment)
            .unwrap();
        assert_eq!(fs::read(&attachment).unwrap(), b"SYNTHETIC-ATTACHMENT");
        assert!(
            v.export_credential_attachment(item_id, 0, &attachment)
                .is_err()
        );
        let export = dir.path().join("roundtrip.1pux");
        v.export_credential_original(item_id, &export).unwrap();
        assert_eq!(fs::read(export).unwrap(), *original);
        v.backup(&dir.path().join("backup")).unwrap();
        drop(v);
        let v = Vault::unlock(&dir.path().join("vault"), PASSWORD).unwrap();
        assert_eq!(v.preview_onepassword(&import).unwrap().duplicates, 1);
        assert_eq!(
            v.credential_details(item_id).unwrap().title,
            "Synthetic Mail"
        );
        let restored = Vault::restore(
            &dir.path().join("backup"),
            &dir.path().join("restored"),
            PASSWORD,
        )
        .unwrap();
        assert!(
            restored
                .credential_details(item_id)
                .unwrap()
                .fields
                .iter()
                .any(|f| *f.value == SECRET)
        );
        no_plaintext(&dir.path().join("vault"));
        no_plaintext(&dir.path().join("backup"));
        no_plaintext(&dir.path().join("restored"));
    }
    #[test]
    fn imports_two_and_three_underscore_attachments_with_original_names_and_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = vault(&dir);
        for (index, separator) in ["__", "___"].iter().enumerate() {
            let mut item = login();
            item["uuid"] = json!(format!("item{index}"));
            item["details"]["documentAttributes"] =
                json!({"documentId":"doc1","fileName":"_Key ___ backup 🗝.txt"});
            // Cover section attachments and the alternate documentID spelling too.
            item["details"]["sections"][0]["fields"].as_array_mut().unwrap().push(
                json!({"title":"Recovery file","value":{"file":{"documentID":"doc2","fileName":"recovery.txt"}}}),
            );
            let document_path = format!("files/doc1{separator}_Key ___ backup 🗝.txt");
            let attachment_path = format!("files/doc2{separator}recovery.txt");
            let original = archive(
                &data(vec![item]),
                &[
                    (&document_path, b"SYNTHETIC-DOCUMENT"),
                    (&attachment_path, b"SYNTHETIC-ATTACHMENT"),
                    ("files/doc10__unrelated.txt", b"unrelated"),
                ],
            );
            // Use the file entry point as the native picker does.
            let export = dir.path().join(format!("export{index}.1PUX"));
            fs::write(&export, original.as_slice()).unwrap();
            let import = OnePasswordImport::read(&export).unwrap();
            assert_eq!(v.preview_onepassword(&import).unwrap().new, 1);
            assert_eq!(v.import_onepassword(&import).unwrap().new, 1);
            let item_id: i64 =
                v.db.query_row(
                    "SELECT item_id FROM credential_record WHERE item_uuid=?",
                    [format!("item{index}")],
                    |r| r.get(0),
                )
                .unwrap();
            let details = v.credential_details(item_id as u64).unwrap();
            assert_eq!(details.attachments.len(), 2);
            assert_eq!(details.attachments[0].name, "_Key ___ backup 🗝.txt");
            assert_eq!(details.attachments[1].name, "recovery.txt");
            for (attachment, expected) in
                [b"SYNTHETIC-DOCUMENT".as_slice(), b"SYNTHETIC-ATTACHMENT"]
                    .iter()
                    .enumerate()
            {
                let destination = dir.path().join(format!("file{index}-{attachment}.txt"));
                v.export_credential_attachment(item_id as u64, attachment, &destination)
                    .unwrap();
                assert_eq!(fs::read(destination).unwrap(), *expected);
            }
            let recovered = dir.path().join(format!("recovered{index}.1pux"));
            v.export_credential_original(item_id as u64, &recovered)
                .unwrap();
            assert_eq!(fs::read(recovered).unwrap(), *original);
            assert_eq!(v.import_onepassword(&import).unwrap().duplicates, 1);
        }
    }
    #[test]
    fn attachment_resolution_rejects_ambiguous_names_and_id_prefix_collisions() {
        let mut item = login();
        item["details"]["documentAttributes"] = json!({"documentId":"doc1"});
        let input = data(vec![item]);
        for extras in [
            vec![
                ("files/doc1__key.txt", b"a".as_slice()),
                ("files/doc1___key.txt", b"b"),
            ],
            vec![
                ("files/doc1__key.txt", b"a".as_slice()),
                ("files/doc1__other.txt", b"b"),
            ],
            vec![("files/doc10__key.txt", b"a".as_slice())],
            vec![("files/doc1_key.txt", b"a".as_slice())],
        ] {
            assert!(OnePasswordImport::from_bytes(archive(&input, &extras)).is_err());
        }
    }

    #[test]
    fn duplicates_are_idempotent_and_changed_versions_do_not_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = vault(&dir);
        let first = batch(vec![login()]);
        v.import_onepassword(&first).unwrap();
        assert_eq!(v.import_onepassword(&first).unwrap().duplicates, 1);
        let original_id = v.collection("", false).unwrap().items[0].id;
        let mut changed = login();
        changed["details"]["loginFields"][1]["value"] = json!("replacement");
        let second = batch(vec![changed]);
        let preview = v.preview_onepassword(&second).unwrap();
        assert_eq!((preview.new, preview.changed), (0, 1));
        v.import_onepassword(&second).unwrap();
        assert_eq!(v.collection("", false).unwrap().total, 2);
        assert!(
            v.credential_details(original_id)
                .unwrap()
                .fields
                .iter()
                .any(|f| *f.value == SECRET)
        );
        assert_eq!(v.import_onepassword(&first).unwrap().duplicates, 1);
        assert_eq!(v.import_onepassword(&second).unwrap().duplicates, 1);
        assert_eq!(
            v.db.query_row("SELECT count(*) FROM credential_import", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            2
        );
    }
    #[test]
    fn item_identity_is_scoped_to_account_and_vault() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = vault(&dir);
        let mut input = data(vec![login()]);
        let mut second = input["accounts"][0]["vaults"][0].clone();
        second["attrs"]["uuid"] = json!("vault2");
        input["accounts"][0]["vaults"]
            .as_array_mut()
            .unwrap()
            .push(second);
        let mut account = input["accounts"][0].clone();
        account["attrs"]["uuid"] = json!("account2");
        input["accounts"].as_array_mut().unwrap().push(account);
        let import = OnePasswordImport::from_bytes(archive(&input, &[])).unwrap();
        assert_eq!(v.import_onepassword(&import).unwrap().new, 4);
        assert_eq!(v.collection("", false).unwrap().total, 4);
    }
    #[test]
    fn local_search_finds_all_collection_kinds_and_imported_categories() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = vault(&dir);
        let mut entries = Vec::new();
        for (index, category) in ["001", "002", "003", "004", "999"].iter().enumerate() {
            let mut entry = login();
            entry["uuid"] = json!(format!("search-{index}"));
            entry["overview"]["title"] = json!(format!("Instagram {index}"));
            entry["categoryUuid"] = json!(category);
            if index == 4 {
                entry["state"] = json!("archived");
            }
            entries.push(entry);
        }
        v.import_onepassword(&batch(entries)).unwrap();
        let note = v
            .save_note(None, "Instagram reminder", "Synthetic account reference")
            .unwrap();
        let path = dir.path().join("Instagram guide.txt");
        fs::write(&path, "UniqueIndexedContent").unwrap();
        let document = v
            .import_document(&path, "filename-match", crate::DocumentClass::Unclassified)
            .unwrap();
        let path = dir.path().join("Other.txt");
        fs::write(&path, "Instagram content match").unwrap();
        let content = v
            .import_document(&path, "content-match", crate::DocumentClass::Personal)
            .unwrap();
        v.enable_text_search(content).unwrap();
        let path = dir.path().join("Instagram recovery.txt");
        fs::write(&path, "HiddenRecoverySecret").unwrap();
        let restricted = v
            .import_document(
                &path,
                "credential-document",
                crate::DocumentClass::Credential,
            )
            .unwrap();
        // Exercise the same collection query as the main Search page, including
        // existing imports after unlocking, with no AI or provider dependency.
        drop(v);
        let v = Vault::unlock(&dir.path().join("vault"), PASSWORD).unwrap();
        for query in ["Instagram", "  INSTAGRAM  ", "insta"] {
            let result = v.collection(query, false).unwrap();
            assert_eq!(result.items.len(), 9, "{query}");
            assert!(result.get(note).is_some());
            assert!(result.get(document).is_some());
            assert!(result.get(content).is_some());
            assert!(result.get(restricted).is_some());
            assert_eq!(
                result
                    .items
                    .iter()
                    .filter(|item| matches!(item.content, crate::Content::Credential { .. }))
                    .count(),
                5
            );
        }
        assert!(
            v.collection("UniqueIndexedContent", false)
                .unwrap()
                .items
                .is_empty()
        );
        assert!(
            v.collection("doesnotexist", false)
                .unwrap()
                .items
                .is_empty()
        );
        assert!(v.collection("86402", false).unwrap().items.is_empty());
        assert!(
            v.collection("HiddenRecoverySecret", false)
                .unwrap()
                .items
                .is_empty()
        );
        assert_eq!(v.collection("personal", false).unwrap().items.len(), 5);
        v.db.execute(
            "UPDATE collection_item SET deleted_at='synthetic' WHERE local_id=?",
            [sql_id(document).unwrap()],
        )
        .unwrap();
        assert_eq!(v.collection("Instagram", false).unwrap().items.len(), 8);
    }
    #[test]
    fn knowledge_map_exposes_credential_metadata_without_protected_fields() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = vault(&dir);
        v.import_onepassword(&batch(vec![login()])).unwrap();
        let graph = v.knowledge_map().unwrap();
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].kind, crate::KnowledgeKind::Credential);
        assert_eq!(graph.nodes[0].status, crate::KnowledgeStatus::Restricted);
        let displayed = format!("{graph:?}");
        for secret in [
            SECRET,
            "86402",
            "001234",
            "TESTSECRET",
            "private note",
            "tester",
        ] {
            assert!(!displayed.contains(secret));
        }
        assert!(graph.edges.is_empty());
    }
    #[test]
    fn credentials_never_enter_search_content_ai_jobs_or_agent_scope() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = vault(&dir);
        v.import_onepassword(&batch(vec![login()])).unwrap();
        let item = v.collection("", false).unwrap().items[0].id;
        assert_eq!(
            v.collection("synthetic mail", false).unwrap().items.len(),
            1
        );
        assert_eq!(v.collection("personal", false).unwrap().items.len(), 1);
        for secret in ["86402", "001234", "TESTSECRET", "private note", "tester"] {
            assert!(v.collection(secret, false).unwrap().items.is_empty());
        }
        for table in ["source_segment", "assertion", "job", "document_evaluation"] {
            assert_eq!(
                v.db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
        assert!(v.next_automatic_document().unwrap().is_none());
        v.set_automatic_evaluation(false).unwrap();
        v.set_automatic_evaluation(true).unwrap();
        assert!(v.next_automatic_document().unwrap().is_none());
        assert!(v.begin_evaluation(item, false).is_err());
        assert!(v.enable_text_search(item).is_err());
        assert!(v.prepare_extraction(item, "test").is_err());
        assert!(
            v.documents_search(&v.shareable_scope().unwrap(), "Synthetic", 10)
                .unwrap()["hits"]
                .as_array()
                .is_some_and(|a| a.is_empty())
        );
        let source: String =
            v.db.query_row(
                "SELECT source_id FROM collection_item WHERE local_id=?",
                [sql_id(item).unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            crate::vault::segment(&v.db.unchecked_transaction().unwrap(), &source, 0, SECRET)
                .is_err()
        );
    }
    #[test]
    fn storage_failure_rolls_back_the_entire_import() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = vault(&dir);
        v.db.execute_batch("CREATE TRIGGER synthetic_failure BEFORE INSERT ON credential_record WHEN NEW.item_uuid='item2' BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
        let mut second = login();
        second["uuid"] = json!("item2");
        assert!(v.import_onepassword(&batch(vec![login(), second])).is_err());
        for table in [
            "credential_import",
            "credential_record",
            "source",
            "collection_item",
            "local_change",
        ] {
            assert_eq!(
                v.db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
    }
    #[test]
    fn rejects_broken_archives_unsupported_versions_and_missing_attachments() {
        for bytes in [
            Zeroizing::new(b"not a zip".to_vec()),
            zip_files(&[
                ("export.attributes", br#"{"version":99}"#),
                ("export.data", b"{}"),
            ]),
            zip_files(&[
                ("export.attributes", br#"{"version":3}"#.as_slice()),
                ("export.data", b"{secret malformed"),
            ]),
            archive(&json!({"accounts":"wrong"}), &[]),
            archive(&data(vec![login(), login()]), &[]),
        ] {
            assert!(OnePasswordImport::from_bytes(bytes).is_err());
        }
        let mut item = login();
        item["details"]["documentAttributes"] = json!({"documentId":"missing"});
        assert!(OnePasswordImport::from_bytes(archive(&data(vec![item]), &[])).is_err());
        let mut item = login();
        item["details"]["sections"][0]["fields"] = json!("broken");
        assert!(OnePasswordImport::from_bytes(archive(&data(vec![item]), &[])).is_err());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.1pux");
        fs::write(&path, b"SYNTHETIC-SECRET").unwrap();
        let error = OnePasswordImport::read(&path).err().unwrap();
        assert!(!format!("{error:?}").contains("SYNTHETIC"));
        assert!(OnePasswordImport::read(&dir.path().join("passwords.csv")).is_err());
    }
    #[test]
    fn refuses_unsafe_paths_and_expansion_limits_without_unpacking() {
        for name in [
            "../outside.txt",
            "/absolute.txt",
            "files/../../outside.txt",
            "files\\outside.txt",
        ] {
            assert!(
                OnePasswordImport::from_bytes(archive(&data(vec![login()]), &[(name, b"x")]))
                    .is_err()
            );
        }
        let bytes = vec![b' '; MAX_DATA as usize + 1];
        assert!(
            OnePasswordImport::from_bytes(zip_files(&[
                ("export.attributes", br#"{"version":3}"#.as_slice()),
                ("export.data", &bytes)
            ]))
            .is_err()
        );
    }
    #[test]
    fn supports_empty_exports_password_notes_and_unknown_categories_without_data_loss() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = vault(&dir);
        assert_eq!(v.import_onepassword(&batch(vec![])).unwrap().total, 0);
        let mut items = Vec::new();
        for (i, category) in ["003", "005", "999"].iter().enumerate() {
            let mut item = login();
            item["uuid"] = json!(format!("item{i}"));
            item["categoryUuid"] = json!(category);
            item["details"] =
                json!({"password":SECRET,"notesPlain":"note","futureField":{"a":"future value"}});
            items.push(item);
        }
        assert_eq!(v.import_onepassword(&batch(items)).unwrap().new, 3);
        for item in &v.collection("", false).unwrap().items {
            assert!(
                v.credential_details(item.id)
                    .unwrap()
                    .fields
                    .iter()
                    .any(|f| *f.value == SECRET)
            );
        }
    }
    #[test]
    fn version_five_migration_preserves_existing_documents_and_notes() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = vault(&dir);
        v.save_note(None, "Existing", "original content").unwrap();
        v.db.execute_batch(
            "DROP TABLE knowledge_view; DROP TABLE knowledge_position; DROP TABLE import_control; DROP TABLE import_request; DROP TABLE import_budget; DROP TABLE import_step_cache; DROP TABLE import_step_progress; DROP TABLE import_progress; DROP TABLE data_recent; DROP TABLE document_folder; DROP TABLE credential_record; DROP TABLE credential_import; PRAGMA user_version=5;",
        )
        .unwrap();
        drop(v);
        let mut v = Vault::unlock(&dir.path().join("vault"), PASSWORD).unwrap();
        assert_eq!(v.collection("original", false).unwrap().items.len(), 1);
        v.import_onepassword(&batch(vec![login()])).unwrap();
        v.verify().unwrap();
        assert_eq!(v.collection("", false).unwrap().total, 2);
    }
}
