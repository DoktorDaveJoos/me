//! Login-specific reads and edits. Original 1PUX bytes and import fingerprints
//! stay immutable; editable data remains credential-scoped and outside FTS/AI.
use crate::{
    Error, Result, Vault,
    credentials::{json, optional_array, optional_text},
    vault::{id, sql_id},
};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json as value};
use std::collections::BTreeSet;
use zeroize::Zeroizing;

#[derive(Clone)]
pub struct LoginSummary {
    pub id: u64,
    pub category: String,
    pub title: String,
    pub vault: String,
    pub website: String,
    pub archived: bool,
    pub favorite: bool,
    pub versions: usize,
    pub version: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LoginValueKind {
    Text,
    Json,
    Tags,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LoginPresentation {
    Value,
    Label,
    Number,
    Tags,
    History,
}

// Intentionally no Debug/Serialize for values or drafts.
pub struct LoginField {
    pub key: String,
    pub label: String,
    pub section: String,
    pub value: Zeroizing<String>,
    pub concealed: bool,
    pub kind: LoginValueKind,
    pub editable: bool,
    pub multiline: bool,
    pub presentation: LoginPresentation,
    /// 1PUX time is when this password became non-current, not when it was created.
    pub used_until: Option<i64>,
    pub used_until_date: Option<String>,
}
pub struct LoginDetails {
    pub native_kind: Option<crate::CredentialKind>,
    pub credential: crate::CredentialDetails,
    pub revision: i64,
    pub favorite: bool,
    pub fields: Vec<LoginField>,
}
pub struct LoginUpdate {
    pub revision: i64,
    pub title: String,
    pub favorite: bool,
    /// Only changed fields need to be sent. Keys are issued by login_details.
    pub fields: Vec<(String, Zeroizing<String>)>,
}

fn text_field(key: String, label: &str, section: &str, text: &str, concealed: bool) -> LoginField {
    LoginField {
        key,
        label: label.into(),
        section: section.into(),
        value: Zeroizing::new(text.into()),
        concealed,
        kind: LoginValueKind::Text,
        editable: true,
        multiline: text.contains('\n'),
        presentation: LoginPresentation::Value,
        used_until: None,
        used_until_date: None,
    }
}
fn login_fields(item: &Value) -> Result<Vec<LoginField>> {
    let mut fields = Vec::new();
    let details = &item["details"];
    let login = optional_array(details, "loginFields")?;
    for designation in ["username", "password"] {
        if let Some((i, field)) = login
            .iter()
            .enumerate()
            .find(|(_, f)| f["designation"].as_str() == Some(designation))
        {
            fields.push(text_field(
                format!("/details/loginFields/{i}/value"),
                if designation == "username" {
                    "Username"
                } else {
                    "Password"
                },
                "Login",
                optional_text(field, "value")?,
                designation == "password",
            ));
        } else if designation == "password" && details["password"].is_string() {
            fields.push(text_field(
                "/details/password".into(),
                "Password",
                "Login",
                optional_text(details, "password")?,
                true,
            ));
        } else {
            fields.push(text_field(
                format!("${designation}"),
                if designation == "username" {
                    "Username"
                } else {
                    "Password"
                },
                "Login",
                "",
                designation == "password",
            ));
        }
    }
    let primary_keys: BTreeSet<_> = fields.iter().map(|f| f.key.clone()).collect();
    if let Some(password) = details["password"].as_str()
        && password != fields[1].value.as_str()
        && !primary_keys.contains("/details/password")
    {
        fields.push(text_field(
            "/details/password".into(),
            "Additional password",
            "Other details",
            password,
            true,
        ));
    }
    for (i, field) in login.iter().enumerate() {
        if !primary_keys.contains(&format!("/details/loginFields/{i}/value")) {
            let label = optional_text(field, "name")?;
            fields.push(text_field(
                format!("/details/loginFields/{i}/value"),
                if label.is_empty() {
                    "Form field"
                } else {
                    label
                },
                "Web form details",
                optional_text(field, "value")?,
                true,
            ));
        }
    }
    let primary = optional_text(&item["overview"], "url")?;
    fields.push(text_field(
        "/overview/url".into(),
        "Website",
        "Websites",
        primary,
        false,
    ));
    for (i, url) in optional_array(&item["overview"], "urls")?
        .iter()
        .enumerate()
    {
        let text = optional_text(url, "url")?;
        if text != primary {
            let label = optional_text(url, "label")?;
            fields.push(text_field(
                format!("/overview/urls/{i}/url"),
                if label.is_empty() { "Website" } else { label },
                "Websites",
                text,
                false,
            ));
        }
    }
    for (s, section) in optional_array(details, "sections")?.iter().enumerate() {
        let heading = optional_text(section, "title")?;
        for (i, field) in optional_array(section, "fields")?.iter().enumerate() {
            let mut key = format!("/details/sections/{s}/fields/{i}/value");
            let mut data = &field["value"];
            let mut concealed = true;
            let mut presentation = LoginPresentation::Value;
            if let Some(object) = data.as_object().filter(|o| o.len() == 1) {
                let (name, inner) = object.iter().next().unwrap();
                key.push('/');
                key.push_str(&name.replace('~', "~0").replace('/', "~1"));
                concealed = !matches!(
                    name.as_str(),
                    "string" | "email" | "url" | "phone" | "date" | "monthYear" | "menu" | "number"
                );
                presentation = match name.as_str() {
                    "number" => LoginPresentation::Number,
                    "menu" => LoginPresentation::Label,
                    _ => LoginPresentation::Value,
                };
                data = inner;
            }
            concealed |= field["guarded"].as_bool().unwrap_or(false);
            let title = optional_text(field, "title")?;
            if !concealed
                && matches!(
                    title.to_ascii_lowercase().as_str(),
                    "label" | "labels" | "account label"
                )
            {
                presentation = LoginPresentation::Label;
            }
            let mut field = text_field(
                key,
                if title.is_empty() {
                    "Custom field"
                } else {
                    title
                },
                if heading.is_empty() {
                    "Other details"
                } else {
                    heading
                },
                "",
                concealed,
            );
            field.presentation = if concealed {
                LoginPresentation::Value
            } else {
                presentation
            };
            field.kind = if data.is_string() {
                LoginValueKind::Text
            } else {
                LoginValueKind::Json
            };
            field.value = Zeroizing::new(if let Some(s) = data.as_str() {
                s.into()
            } else {
                serde_json::to_string(data).map_err(|_| Error::Format)?
            });
            field.multiline = field.value.contains('\n');
            fields.push(field);
        }
    }
    let mut notes = text_field(
        "/details/notesPlain".into(),
        "Notes",
        "Notes",
        optional_text(details, "notesPlain")?,
        false,
    );
    notes.multiline = true;
    fields.push(notes);
    let tags = optional_array(&item["overview"], "tags")?
        .iter()
        .map(|v| v.as_str().ok_or(Error::Format))
        .collect::<Result<Vec<_>>>()?
        .join("\n");
    let mut tags = text_field("/overview/tags".into(), "Tags", "Tags", &tags, false);
    tags.kind = LoginValueKind::Tags;
    tags.presentation = LoginPresentation::Tags;
    tags.multiline = true;
    fields.push(tags);
    let mut history = Vec::new();
    let mut recovery = text_field(
        "$recovery_codes".into(),
        "Additional recovery codes",
        "Recovery",
        "",
        true,
    );
    recovery.multiline = true;
    fields.push(recovery);
    for (i, entry) in optional_array(details, "passwordHistory")?
        .iter()
        .enumerate()
    {
        let mut field = text_field(
            format!("/details/passwordHistory/{i}/value"),
            "Previous password",
            "Password history",
            optional_text(entry, "value")?,
            true,
        );
        field.editable = false;
        field.presentation = LoginPresentation::History;
        field.used_until = entry["time"]
            .as_i64()
            .filter(|time| (0..=253_402_300_799).contains(time));
        history.push(field);
    }
    history.sort_by_key(|f| std::cmp::Reverse(f.used_until));
    fields.extend(history);
    Ok(fields)
}

impl Vault {
    /// No 200-result collection cap: every imported Login has a row, including
    /// archives and retained import versions. Other 1Password categories stay intact.
    pub fn logins(&self) -> Result<Vec<LoginSummary>> {
        let mut stmt = self.db.prepare("SELECT i.local_id,i.title,c.vault_name,c.archived,i.pinned,count(*) OVER (PARTITION BY c.account_uuid,c.vault_uuid,c.item_uuid),row_number() OVER (PARTITION BY c.account_uuid,c.vault_uuid,c.item_uuid ORDER BY i.local_id),COALESCE(NULLIF(json_extract(c.raw_json,'$.overview.url'),''),json_extract(c.raw_json,'$.overview.urls[0].url'),json_extract(c.raw_json,'$.fields.website'),''),c.category FROM collection_item i JOIN credential_record c ON c.item_id=i.local_id WHERE i.deleted_at IS NULL AND i.kind='credential' AND (c.category='001' OR c.format='me-v1') ORDER BY i.title COLLATE NOCASE,c.vault_name,i.local_id")?;
        Ok(stmt
            .query_map([], |r| {
                Ok(LoginSummary {
                    id: r.get::<_, i64>(0)? as u64,
                    title: r.get(1)?,
                    vault: r.get(2)?,
                    archived: r.get(3)?,
                    favorite: r.get(4)?,
                    versions: r.get::<_, i64>(5)? as usize,
                    version: r.get::<_, i64>(6)? as usize,
                    website: r.get(7)?,
                    category: r.get(8)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?)
    }
    pub fn login_details(&self, item: u64) -> Result<LoginDetails> {
        if let Some(kind) = self.native_kind(item)? {
            return self.native_details(item, kind);
        }
        let credential = self.credential_details(item)?;
        if credential.category != "001" {
            return Err(Error::Validation("This entry is not a login."));
        }
        let (raw, revision, favorite): (String, i64, bool) = self.db.query_row("SELECT c.raw_json,i.revision,i.pinned FROM credential_record c JOIN collection_item i ON i.local_id=c.item_id WHERE c.item_id=? AND i.deleted_at IS NULL", [sql_id(item)?], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        let raw = Zeroizing::new(raw);
        let mut fields = login_fields(&json(raw.as_bytes())?.0)?;
        for field in &mut fields {
            if let Some(unix) = field.used_until {
                field.used_until_date = self.db.query_row(
                    "SELECT strftime('%Y-%m-%d %H:%M UTC',?,'unixepoch')",
                    [unix],
                    |r| r.get(0),
                )?;
            }
        }
        Ok(LoginDetails {
            native_kind: None,
            credential,
            revision,
            favorite,
            fields,
        })
    }
    pub fn update_login(&mut self, item: u64, update: LoginUpdate) -> Result<()> {
        if let Some(kind) = self.native_kind(item)? {
            return self.update_native(item, kind, update);
        }
        let title = update.title.trim();
        if title.is_empty() {
            return Err(Error::Validation("Enter a title for this login."));
        }
        if title.len() > 16_384 {
            return Err(Error::Validation(
                "This login title is too long. Shorten it and try again.",
            ));
        }
        let tx = self.db.transaction()?;
        let stored: Option<(String, i64, String, String)> = tx.query_row("SELECT c.raw_json,i.revision,i.stable_id,i.source_id FROM credential_record c JOIN collection_item i ON i.local_id=c.item_id WHERE c.item_id=? AND i.deleted_at IS NULL AND c.category='001' AND i.kind='credential'", [sql_id(item)?], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).optional()?;
        let (raw, revision, stable, source) =
            stored.ok_or(Error::Validation("This login is no longer available."))?;
        if revision != update.revision {
            return Err(Error::Validation(
                "This login changed. Cancel editing and reopen it before saving.",
            ));
        }
        let raw = Zeroizing::new(raw);
        let mut data = json(raw.as_bytes())?;
        let available = login_fields(&data.0)?;
        let mut seen = BTreeSet::new();
        let mut old_passwords = Vec::new();
        for (key, text) in &update.fields {
            if text.len() > 1_048_576 {
                return Err(Error::Validation("A login field exceeds 1 MB."));
            }
            if !seen.insert(key) {
                return Err(Error::Validation("A login field was edited twice."));
            }
            let field = available
                .iter()
                .find(|f| &f.key == key && f.editable)
                .ok_or(Error::Validation("This login field cannot be edited."))?;
            if field.value.as_str() == text.as_str() {
                continue;
            }
            if field.key == available[1].key && !field.value.is_empty() {
                old_passwords.push(field.value.clone());
            }
            if key == "$recovery_codes" {
                if data.0["details"]["sections"].is_null() {
                    data.0["details"]["sections"] = value!([]);
                }
                data.0["details"]["sections"].as_array_mut().ok_or(Error::Format)?.push(value!({"title":"Recovery", "fields":[{"title":"Recovery codes","value":{"concealed":text.as_str()}}]}));
                continue;
            }
            if key == "$username" || key == "$password" {
                if data.0["details"]["loginFields"].is_null() {
                    data.0["details"]["loginFields"] = value!([]);
                }
                let designation = key.trim_start_matches('$');
                data.0["details"]["loginFields"].as_array_mut().ok_or(Error::Format)?.push(value!({"designation":designation,"name":designation,"fieldType":if designation=="password" {"P"} else {"T"},"value":text.as_str()}));
                continue;
            }
            // overview.url and its repeated urls[] entry represent one website.
            if key == "/overview/url"
                && let Some(urls) = data.0["overview"]["urls"].as_array_mut()
            {
                for url in urls {
                    if url["url"].as_str() == Some(field.value.as_str()) {
                        url["url"] = Value::String(text.to_string());
                    }
                }
            }
            // Some exports contain a second copy of the current password.
            if field.key == available[1].key
                && key != "/details/password"
                && data.0["details"]["password"].as_str() == Some(field.value.as_str())
            {
                data.0["details"]["password"] = Value::String(text.to_string());
            }
            let replacement = match field.kind {
                LoginValueKind::Text => Value::String(text.to_string()),
                LoginValueKind::Tags => Value::Array(
                    text.lines()
                        .filter(|s| !s.is_empty())
                        .map(|s| Value::String(s.into()))
                        .collect(),
                ),
                LoginValueKind::Json => {
                    let mut parsed = json(text.as_bytes()).map_err(|_| {
                        Error::Validation(if field.presentation == LoginPresentation::Number {
                            "Enter a valid number for this field."
                        } else {
                            "Enter valid JSON for the structured field."
                        })
                    })?;
                    let original = data.0.pointer(key).ok_or(Error::Format)?;
                    let same_type = matches!(
                        (original, &parsed.0),
                        (Value::Object(_), Value::Object(_))
                            | (Value::Array(_), Value::Array(_))
                            | (Value::Number(_), Value::Number(_))
                            | (Value::Bool(_), Value::Bool(_))
                            | (Value::Null, Value::Null)
                    );
                    if !same_type {
                        return Err(Error::Validation(
                            if field.presentation == LoginPresentation::Number {
                                "Enter a valid number for this field."
                            } else {
                                "Keep the original type of the structured field."
                            },
                        ));
                    }
                    std::mem::take(&mut parsed.0)
                }
            };
            if let Some(target) = data.0.pointer_mut(key) {
                *target = replacement;
            } else {
                match key.as_str() {
                    "/overview/url" => data.0["overview"]["url"] = replacement,
                    "/overview/tags" => data.0["overview"]["tags"] = replacement,
                    "/details/notesPlain" => data.0["details"]["notesPlain"] = replacement,
                    _ => return Err(Error::Format),
                }
            }
        }
        let now: i64 = tx.query_row("SELECT unixepoch()", [], |r| r.get(0))?;
        if !old_passwords.is_empty() {
            if data.0["details"]["passwordHistory"].is_null() {
                data.0["details"]["passwordHistory"] = value!([]);
            }
            let history = data.0["details"]["passwordHistory"]
                .as_array_mut()
                .ok_or(Error::Format)?;
            for password in old_passwords {
                history.push(value!({"value":password.as_str(),"time":now}));
            }
        }
        data.0["overview"]["title"] = Value::String(title.into());
        data.0["favIndex"] = value!(if update.favorite { 1 } else { 0 });
        data.0["updatedAt"] = value!(now);
        // Validate the complete display model before any write.
        let _ = login_fields(&data.0)?;
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
        tx.execute("INSERT INTO local_change(operation_id,item_id,revision,kind,payload_json,recorded_at) VALUES(?,?,?,'credential_edit','{}',strftime('%Y-%m-%dT%H:%M:%fZ','now'))", params![id(),stable,revision+1])?;
        tx.commit()?;
        Ok(())
    }
}
