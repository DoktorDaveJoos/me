//! Version 1: password wrapping key -> independent random database/object keys.
use crate::{Error, Result};
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};
use zeroize::Zeroizing;

const MAGIC: &[u8] = b"MEOBJ001";
// Fixed per format version, never accepted from attacker-controlled metadata.
const MEMORY_KIB: u32 = 65536;
const ITERATIONS: u32 = 3;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Header {
    pub version: u32,
    pub vault_id: String,
    pub salt: [u8; 16],
    pub wrapped_keys: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<crate::account::AccountBinding>,
}

pub(crate) type Keys = Zeroizing<[u8; 64]>;

pub(crate) fn wrapping_key(password: &str, salt: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
    if password.len() > 4096 {
        return Err(Error::Validation("This password is too long."));
    }
    let mut key = Zeroizing::new([0u8; 32]);
    let params = Params::new(MEMORY_KIB, ITERATIONS, 1, Some(32)).map_err(|_| Error::Format)?;
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, key.as_mut())
        .map_err(|_| Error::Authentication)?;
    Ok(key)
}

impl Header {
    pub fn create(password: &str) -> Result<(Self, Keys)> {
        // Keep onboarding permissive: length only, no character-class rules.
        if password.chars().count() < 10 {
            return Err(Error::Validation(
                "Use at least 10 characters for your vault password.",
            ));
        }
        let mut salt = [0u8; 16];
        let mut keys = Zeroizing::new([0u8; 64]);
        OsRng.try_fill_bytes(&mut salt).map_err(|_| Error::Format)?;
        OsRng
            .try_fill_bytes(keys.as_mut())
            .map_err(|_| Error::Format)?;
        let vault_id = uuid::Uuid::new_v4().to_string();
        let key = wrapping_key(password, &salt)?;
        let wrapped_keys = seal(
            key.as_ref(),
            keys.as_ref(),
            format!("me-keys-v1:{vault_id}").as_bytes(),
        )?;
        Ok((
            Self {
                version: 1,
                vault_id,
                salt,
                wrapped_keys,
                account: None,
            },
            keys,
        ))
    }
    pub fn read(path: &Path) -> Result<Self> {
        let bytes = read_bounded(path, 8192)?;
        let header: Self = serde_json::from_slice(&bytes).map_err(|_| Error::Format)?;
        if !matches!(header.version, 1 | 2)
            || (header.version == 2 && header.account.is_none())
            || (header.version == 1 && header.account.is_some())
            || uuid::Uuid::parse_str(&header.vault_id).is_err()
            || header.wrapped_keys.len() != 112
        {
            return Err(Error::Format);
        }
        Ok(header)
    }
    pub fn unlock(&self, password: &str) -> Result<Keys> {
        let key = wrapping_key(password, &self.salt)?;
        let raw = open(
            key.as_ref(),
            &self.wrapped_keys,
            format!("me-keys-v1:{}", self.vault_id).as_bytes(),
        )?;
        if raw.len() != 64 {
            return Err(Error::Format);
        }
        let mut keys = Zeroizing::new([0u8; 64]);
        keys.copy_from_slice(&raw);
        Ok(keys)
    }
    pub fn write(&self, path: &Path) -> Result<()> {
        atomic_write(path, &serde_json::to_vec(self).map_err(|_| Error::Format)?)
    }
}

pub(crate) fn seal(key: &[u8], plaintext: &[u8], context: &[u8]) -> Result<Vec<u8>> {
    let cipher = XChaCha20Poly1305::new_from_slice(key).map_err(|_| Error::Format)?;
    let mut nonce = [0u8; 24];
    OsRng
        .try_fill_bytes(&mut nonce)
        .map_err(|_| Error::Format)?;
    let encrypted = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: context,
            },
        )
        .map_err(|_| Error::Authentication)?;
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(&nonce);
    bytes.extend(encrypted);
    Ok(bytes)
}

pub(crate) fn open(key: &[u8], data: &[u8], context: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    if data.len() < 48 || &data[..8] != MAGIC {
        return Err(Error::Format);
    }
    let cipher = XChaCha20Poly1305::new_from_slice(key).map_err(|_| Error::Format)?;
    cipher
        .decrypt(
            XNonce::from_slice(&data[8..32]),
            Payload {
                msg: &data[32..],
                aad: context,
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| Error::Authentication)
}

pub(crate) fn read_bounded(path: &Path, limit: u64) -> Result<Zeroizing<Vec<u8>>> {
    let file = File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(Error::Validation("Choose a regular file."));
    }
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(Error::Validation("This file exceeds the size limit."));
    }
    Ok(bytes)
}

/// The destination must not exist. Flush bytes, publish without clobbering, then
/// flush the directory. A failed call never silently replaces an existing file.
pub(crate) fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or(Error::Format)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(data)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(path)
        .map_err(|err| Error::Io(err.error))?;
    File::open(parent)?.sync_all()?;
    Ok(())
}
