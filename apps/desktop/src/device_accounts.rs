//! Device account selection, not authentication. Only validated relative slots
//! are persisted. Every vault still requires its master password to open.
//! All filesystem work belongs on a background worker.
use me_core::{Vault, account};
use me_protocol::AccountResponse;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const PRIMARY: &str = "primary";
const MAX_PROFILES: usize = 256;

pub(super) struct Selection {
    pub root: PathBuf,
    pub signed_out: bool,
}
fn base_dir(base: &Path) -> Result<&Path, String> {
    base.parent()
        .ok_or_else(|| "Couldn't locate account settings.".into())
}
fn config(base: &Path) -> Result<PathBuf, String> {
    Ok(base_dir(base)?.join("device-account.json"))
}
fn valid_slot(slot: &str) -> bool {
    slot == PRIMARY || uuid::Uuid::parse_str(slot).is_ok_and(|id| id.to_string() == slot)
}
fn directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_dir() => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err("Couldn't access the local account folder.".into()),
    }
}
fn slot_root(base: &Path, slot: &str) -> Result<PathBuf, String> {
    if !valid_slot(slot) {
        return Err("Invalid saved account selection.".into());
    }
    if slot == PRIMARY {
        return Ok(base.to_owned());
    }
    let profiles = base_dir(base)?.join("accounts");
    directory(&profiles)?;
    let profile = profiles.join(slot);
    directory(&profile)?;
    Ok(profile.join("vault"))
}
fn root_slot(base: &Path, root: &Path) -> Result<String, String> {
    if root == base {
        return Ok(PRIMARY.into());
    }
    let profiles = base_dir(base)?.join("accounts");
    let parts = root
        .strip_prefix(profiles)
        .map_err(|_| "Invalid account location.")?
        .components()
        .collect::<Vec<_>>();
    if parts.len() != 2 || parts[1].as_os_str() != "vault" {
        return Err("Invalid account location.".into());
    }
    let slot = parts[0]
        .as_os_str()
        .to_str()
        .ok_or("Invalid account location.")?;
    if slot == PRIMARY || !valid_slot(slot) {
        return Err("Invalid account location.".into());
    }
    Ok(slot.into())
}
pub(super) fn load(base: &Path) -> Result<Selection, String> {
    let path = config(base)?;
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Selection {
                root: base.to_owned(),
                signed_out: false,
            });
        }
        Err(_) => return Err("Couldn't read the saved account selection.".into()),
    };
    if !meta.file_type().is_file() || meta.len() > 4096 {
        return Err("Invalid saved account selection.".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|_| "Couldn't read account settings.")?
        .take(4097)
        .read_to_end(&mut bytes)
        .map_err(|_| "Couldn't read account settings.")?;
    if bytes.len() > 4096 {
        return Err("Invalid saved account selection.".into());
    }
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid saved account selection.")?;
    if value["version"] != 1 {
        return Err("Invalid saved account selection.".into());
    }
    let (slot, signed_out) = match value.get("active") {
        Some(serde_json::Value::String(slot)) => (slot.as_str(), false),
        Some(serde_json::Value::Null) => (
            value["draft"]
                .as_str()
                .filter(|s| *s != PRIMARY)
                .ok_or("Invalid saved account selection.")?,
            true,
        ),
        _ => return Err("Invalid saved account selection.".into()),
    };
    Ok(Selection {
        root: slot_root(base, slot)?,
        signed_out,
    })
}
fn save(base: &Path, active: Option<&str>, draft: Option<&str>) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let parent = base_dir(base)?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)
        .map_err(|_| "Couldn't create account settings.")?;
    let temporary = parent.join(format!(".account-selection-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(
            serde_json::json!({"version":1,"active":active,"draft":draft})
                .to_string()
                .as_bytes(),
        )?;
        file.sync_all()?;
        fs::rename(&temporary, config(base).map_err(std::io::Error::other)?)?;
        fs::File::open(parent)?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result.map_err(|_| "Couldn't save the account selection. Please try again.".into())
}
pub(super) fn sign_out(base: &Path) -> Result<PathBuf, String> {
    let slot = uuid::Uuid::new_v4().to_string();
    let root = slot_root(base, &slot)?;
    save(base, None, Some(&slot))?;
    Ok(root)
}
pub(super) fn select(base: &Path, root: &Path) -> Result<(), String> {
    let slot = root_slot(base, root)?;
    slot_root(base, &slot)?;
    save(base, Some(&slot), None)
}

/// Locate only the vault identified by an authenticated server response. This
/// also finds staged registrations whose response was lost before binding.
/// Other local accounts are never opened, relabelled or replaced.
pub(super) fn find_local(
    base: &Path,
    server: &str,
    response: &AccountResponse,
) -> Result<Option<PathBuf>, String> {
    let mut candidates = vec![base.to_owned()];
    let profiles = base_dir(base)?.join("accounts");
    directory(&profiles)?;
    match fs::read_dir(&profiles) {
        Ok(entries) => {
            for (index, entry) in entries.enumerate() {
                if index >= MAX_PROFILES {
                    return Err("Too many local account folders to inspect.".into());
                }
                let entry = entry.map_err(|_| "Couldn't inspect local accounts.")?;
                let name = entry.file_name();
                let Some(slot) = name.to_str().filter(|s| *s != PRIMARY && valid_slot(s)) else {
                    continue;
                };
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    candidates.push(slot_root(base, slot)?);
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("Couldn't inspect local accounts.".into()),
    }
    let mut found = None;
    for root in candidates {
        if !Vault::exists(&root).unwrap_or(false) {
            continue;
        }
        let Ok(binding) = account::binding(&root) else {
            continue;
        };
        if binding.is_some_and(|b| b.server != server || b.account.id != response.account.id) {
            continue;
        }
        if account::check_recovery_target(&root, response).is_err() {
            continue;
        }
        if found.is_some() {
            return Err("More than one local vault matches this account. Restore or remove the duplicate before signing in.".into());
        }
        found = Some(root);
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    const PASSWORD: &str = "synthetic-account-password";
    fn fixture(root: &Path, email: &str) -> AccountResponse {
        let code = account::generate_recovery_code().unwrap();
        let request = account::prepare_registration(root, email, PASSWORD, &code).unwrap();
        account::stage_registration(root, PASSWORD, &request).unwrap();
        AccountResponse {
            account: me_protocol::Account {
                id: uuid::Uuid::new_v4().to_string(),
                email: email.into(),
                email_verified: false,
                revision: 1,
            },
            vault: request.vault.clone(),
        }
    }
    #[test]
    fn selected_account_and_explicit_logout_survive_restart() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("vault");
        assert_eq!(load(&base).unwrap().root, base);
        let next = sign_out(&base).unwrap();
        let state = load(&base).unwrap();
        assert!(state.signed_out);
        assert_eq!(state.root, next);
        assert!(!next.exists());
        select(&base, &next).unwrap();
        let state = load(&base).unwrap();
        assert!(!state.signed_out);
        assert_eq!(state.root, next);
        let another = sign_out(&base).unwrap();
        assert_ne!(another, next);
        select(&base, &base).unwrap();
        assert_eq!(load(&base).unwrap().root, base);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(config(&base).unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    #[test]
    fn account_lookup_preserves_separate_vaults_and_recovers_staged_registration() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("vault");
        let server = "http://127.0.0.1:8787";
        let first = fixture(&base, "first@example.test");
        let mut vault = account::open_account(&base, PASSWORD, server, &first, false).unwrap();
        vault
            .save_note(None, "First account", "Preserved first data")
            .unwrap();
        drop(vault);
        let second_root = sign_out(&base).unwrap();
        let second = fixture(&second_root, "second@example.test");
        assert_eq!(
            find_local(&base, server, &second).unwrap(),
            Some(second_root.clone())
        );
        let mut vault =
            account::open_account(&second_root, PASSWORD, server, &second, false).unwrap();
        vault
            .save_note(None, "Second account", "Separate second data")
            .unwrap();
        drop(vault);
        select(&base, &second_root).unwrap();
        sign_out(&base).unwrap();
        assert_eq!(
            find_local(&base, server, &first).unwrap(),
            Some(base.clone())
        );
        assert_eq!(
            find_local(&base, server, &second).unwrap(),
            Some(second_root.clone())
        );
        assert!(
            find_local(&base, "https://other.example.test", &first)
                .unwrap()
                .is_none()
        );
        let vault = Vault::unlock(&base, PASSWORD).unwrap();
        assert_eq!(vault.collection("Preserved", false).unwrap().items.len(), 1);
        assert_eq!(vault.collection("Separate", false).unwrap().items.len(), 0);
        let vault = Vault::unlock(&second_root, PASSWORD).unwrap();
        assert_eq!(vault.collection("Separate", false).unwrap().items.len(), 1);
        assert_eq!(vault.collection("Preserved", false).unwrap().items.len(), 0);
    }
    #[test]
    fn invalid_paths_and_symlink_profiles_are_not_followed() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("vault");
        for value in [
            r#"{"version":1,"active":"../../outside"}"#,
            r#"{"version":1,"active":null,"draft":"primary"}"#,
            r#"{"version":2,"active":"primary"}"#,
        ] {
            fs::write(config(&base).unwrap(), value).unwrap();
            assert!(load(&base).is_err());
        }
        assert!(select(&base, &temp.path().join("outside/vault")).is_err());
        let profile = temp.path().join("accounts");
        std::os::unix::fs::symlink(temp.path(), &profile).unwrap();
        assert!(sign_out(&base).is_err());
    }
}
