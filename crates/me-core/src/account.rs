//! Client-only account derivation and vault-key wrapping. Never linked by me-api.
use crate::{
    Error, Result, Vault,
    crypto::{self, Header, Keys},
};
use me_protocol::{Account, AccountResponse, Recovery, Registration, VaultEnvelope};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Write, path::Path};
use zeroize::Zeroizing;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountBinding {
    pub account: Account,
    pub server: String,
}

pub fn binding(root: &Path) -> Result<Option<AccountBinding>> {
    if !Vault::exists(root)? {
        return Ok(None);
    }
    Ok(Header::read(&root.join("header.json"))?.account)
}

/// Authentication and local encryption use distinct Argon2id inputs/salts.
/// Email is the canonical v1 account identity, never a vault encryption key.
pub fn authentication_secret(email: &str, password: &str) -> Result<Zeroizing<String>> {
    let email = me_protocol::normalize_email(email).map_err(Error::Validation)?;
    if password.len() > 4096 {
        return Err(Error::Validation("This password is too long."));
    }
    let salt = Sha256::digest(format!("me-account-auth-v1:{email}").as_bytes());
    let key = crypto::wrapping_key(password, &salt[..16])?;
    Ok(Zeroizing::new(hex::encode(key.as_ref())))
}

pub fn generate_recovery_code() -> Result<Zeroizing<String>> {
    let mut bytes = Zeroizing::new([0u8; 32]);
    OsRng
        .try_fill_bytes(bytes.as_mut())
        .map_err(|_| Error::Format)?;
    let hex = Zeroizing::new(hex::encode_upper(bytes.as_ref()));
    Ok(Zeroizing::new(
        hex.as_bytes()
            .chunks(8)
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect::<Vec<_>>()
            .join("-"),
    ))
}
fn recovery_key(code: &str, domain: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
    if code.len() > 256 {
        return Err(Error::Validation("Check your recovery code."));
    }
    let normalized = Zeroizing::new(
        code.chars()
            .filter(|c| *c != '-' && !c.is_ascii_whitespace())
            .collect::<String>(),
    );
    if normalized.len() != 64 {
        return Err(Error::Validation("Check your recovery code."));
    }
    let bytes = Zeroizing::new(
        hex::decode(normalized.as_str())
            .map_err(|_| Error::Validation("Check your recovery code."))?,
    );
    let mut hash = Sha256::new();
    hash.update(domain);
    hash.update(&*bytes);
    Ok(Zeroizing::new(hash.finalize().into()))
}
pub fn recovery_secret(code: &str) -> Result<Zeroizing<String>> {
    Ok(Zeroizing::new(hex::encode(
        recovery_key(code, b"me-recovery-auth-v1:")?.as_ref(),
    )))
}
fn envelope(header: &Header, keys: &Keys, code: &str) -> Result<VaultEnvelope> {
    let recovery = recovery_key(code, b"me-recovery-wrap-v1:")?;
    Ok(VaultEnvelope {
        vault_id: header.vault_id.clone(),
        salt: header.salt,
        wrapped_keys: header.wrapped_keys.clone(),
        recovery_keys: crypto::seal(
            recovery.as_ref(),
            keys.as_ref(),
            format!("me-recovery-v1:{}", header.vault_id).as_bytes(),
        )?,
    })
}
pub fn prepare_registration(
    root: &Path,
    email: &str,
    password: &str,
    code: &str,
) -> Result<Registration> {
    let email = me_protocol::normalize_email(email).map_err(Error::Validation)?;
    me_protocol::validate_password(password).map_err(Error::Validation)?;
    let (header, keys) = if Vault::exists(root)? {
        let vault = Vault::unlock(root, password)?;
        if vault.header.account.is_some() {
            return Err(Error::Validation(
                "This vault already has an account. Sign in instead.",
            ));
        }
        (vault.header.clone(), vault.keys.clone())
    } else {
        Header::create(password)?
    };
    Ok(Registration {
        auth_secret: authentication_secret(&email, password)?.to_string(),
        recovery_secret: recovery_secret(code)?.to_string(),
        email,
        vault: envelope(&header, &keys, code)?,
    })
}
pub fn prepare_recovery(
    current: &AccountResponse,
    old_code: &str,
    password: &str,
    new_code: &str,
) -> Result<Recovery> {
    me_protocol::validate_password(password).map_err(Error::Validation)?;
    if !current.vault.valid() {
        return Err(Error::Format);
    }
    let recovery = recovery_key(old_code, b"me-recovery-wrap-v1:")?;
    let raw = crypto::open(
        recovery.as_ref(),
        &current.vault.recovery_keys,
        format!("me-recovery-v1:{}", current.vault.vault_id).as_bytes(),
    )?;
    if raw.len() != 64 {
        return Err(Error::Format);
    }
    let mut keys = Zeroizing::new([0u8; 64]);
    keys.copy_from_slice(&raw);
    let mut salt = [0; 16];
    OsRng.try_fill_bytes(&mut salt).map_err(|_| Error::Format)?;
    let wrapping = crypto::wrapping_key(password, &salt)?;
    let header = Header {
        version: 1,
        vault_id: current.vault.vault_id.clone(),
        salt,
        wrapped_keys: crypto::seal(
            wrapping.as_ref(),
            keys.as_ref(),
            format!("me-keys-v1:{}", current.vault.vault_id).as_bytes(),
        )?,
        account: None,
    };
    Ok(Recovery {
        email: current.account.email.clone(),
        old_recovery_secret: recovery_secret(old_code)?.to_string(),
        auth_secret: authentication_secret(&current.account.email, password)?.to_string(),
        recovery_secret: recovery_secret(new_code)?.to_string(),
        expected_revision: current.account.revision,
        vault: envelope(&header, &keys, new_code)?,
    })
}
/// New registration may create its first local vault. Signing in on another
/// device cannot invent an empty replacement for data that hasn't synced yet.
pub fn open_account(
    root: &Path,
    password: &str,
    server: &str,
    response: &AccountResponse,
    allow_create: bool,
) -> Result<Vault> {
    if !response.vault.valid()
        || uuid::Uuid::parse_str(&response.account.id).is_err()
        || me_protocol::normalize_email(&response.account.email).map_err(Error::Validation)?
            != response.account.email
    {
        return Err(Error::Format);
    }
    let header = Header {
        version: 2,
        vault_id: response.vault.vault_id.clone(),
        salt: response.vault.salt,
        wrapped_keys: response.vault.wrapped_keys.clone(),
        account: Some(AccountBinding {
            account: response.account.clone(),
            server: server.to_owned(),
        }),
    };
    if Vault::exists(root)? {
        if let Some(binding) = binding(root)? {
            if binding.account.id != response.account.id || binding.server != server {
                return Err(Error::Validation(
                    "This device is connected to a different account.",
                ));
            }
            if binding.account.revision > response.account.revision {
                return Err(Error::Validation(
                    "The account response is out of date. Sign in again.",
                ));
            }
        }
        Vault::unlock_with_header(root, password, Some(header))
    } else if allow_create {
        let keys = header.unlock(password)?;
        Vault::create_with_keys(root, header, keys)
    } else {
        Err(Error::Validation(
            "Account verified. Device sync is not available yet. Open ME. on your original device to access your vault.",
        ))
    }
}

pub(crate) fn replace_header(path: &Path, header: &Header) -> Result<()> {
    let parent = path.parent().ok_or(Error::Format)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(&serde_json::to_vec(header).map_err(|_| Error::Format)?)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|err| Error::Io(err.error))?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

/// Save the first encrypted local vault before the network commit. After a
/// dropped response or app restart, sign-in can safely finish the same vault.
pub fn stage_registration(root: &Path, password: &str, request: &Registration) -> Result<()> {
    if Vault::exists(root)? {
        let vault = Vault::unlock(root, password)?;
        if vault.header.vault_id != request.vault.vault_id {
            return Err(Error::Authentication);
        }
        return Ok(());
    }
    let header = Header {
        version: 1,
        vault_id: request.vault.vault_id.clone(),
        salt: request.vault.salt,
        wrapped_keys: request.vault.wrapped_keys.clone(),
        account: None,
    };
    let keys = header.unlock(password)?;
    drop(Vault::create_with_keys(root, header, keys)?);
    Ok(())
}

/// Check an existing local vault before committing a remote recovery.
pub fn check_recovery_target(root: &Path, current: &AccountResponse) -> Result<()> {
    if !Vault::exists(root)? {
        return Ok(());
    }
    let header = Header::read(&root.join("header.json"))?;
    if header.vault_id != current.vault.vault_id
        || header
            .account
            .as_ref()
            .is_some_and(|b| b.account.id != current.account.id)
    {
        return Err(Error::Validation(
            "This account belongs to a different vault. Your local vault has not been changed.",
        ));
    }
    Ok(())
}

pub fn validate_password_envelope(password: &str, response: &AccountResponse) -> Result<()> {
    if !response.vault.valid() {
        return Err(Error::Format);
    }
    let header = Header {
        version: 1,
        vault_id: response.vault.vault_id.clone(),
        salt: response.vault.salt,
        wrapped_keys: response.vault.wrapped_keys.clone(),
        account: None,
    };
    header.unlock(password)?;
    Ok(())
}
