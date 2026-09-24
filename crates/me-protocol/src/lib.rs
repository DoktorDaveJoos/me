//! Account API v1. Contains no vault decryption capabilities.
use serde::{Deserialize, Serialize};
use zeroize::ZeroizeOnDrop;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VaultEnvelope {
    pub vault_id: String,
    pub salt: [u8; 16],
    pub wrapped_keys: Vec<u8>,
    pub recovery_keys: Vec<u8>,
}
impl VaultEnvelope {
    pub fn valid(&self) -> bool {
        uuid::Uuid::parse_str(&self.vault_id).is_ok()
            && self.wrapped_keys.len() == 112
            && self.recovery_keys.len() == 112
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Account {
    pub id: String,
    pub email: String,
    // Validation of email syntax is not proof of ownership.
    pub email_verified: bool,
    pub revision: i64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountResponse {
    pub account: Account,
    pub vault: VaultEnvelope,
}

#[derive(Serialize, Deserialize, ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    #[zeroize(skip)]
    pub email: String,
    pub secret: String,
}
#[derive(Serialize, Deserialize, ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    #[zeroize(skip)]
    pub email: String,
    pub auth_secret: String,
    pub recovery_secret: String,
    #[zeroize(skip)]
    pub vault: VaultEnvelope,
}
#[derive(Serialize, Deserialize, ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
pub struct Recovery {
    #[zeroize(skip)]
    pub email: String,
    pub old_recovery_secret: String,
    pub auth_secret: String,
    pub recovery_secret: String,
    #[zeroize(skip)]
    pub expected_revision: i64,
    #[zeroize(skip)]
    pub vault: VaultEnvelope,
}

/// v1 deliberately supports conventional ASCII email addresses. Local parts are
/// case-insensitive ME account identifiers; aliases and dots are not rewritten.
pub fn normalize_email(value: &str) -> Result<String, &'static str> {
    let email = value.trim().to_ascii_lowercase();
    let Some((local, domain)) = email.split_once('@') else {
        return Err("Enter a valid email address.");
    };
    if email.len() > 254
        || local.is_empty()
        || local.len() > 64
        || !email.is_ascii()
        || local.starts_with('.')
        || local.ends_with('.')
        || local.contains("..")
        || !local
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".!#$%&'*+-/=?^_`{|}~".contains(&c))
        || !domain.contains('.')
        || domain.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        })
    {
        return Err("Enter a valid email address.");
    }
    Ok(email)
}
pub fn valid_secret(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit())
}
pub fn validate_password(value: &str) -> Result<(), &'static str> {
    if value.chars().count() < 10 {
        return Err("Use at least 10 characters for your master password.");
    }
    if value.len() > 4096 {
        return Err("This password is too long.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn email_identity_is_canonical_and_bounded() {
        assert_eq!(
            normalize_email(" Alex+me@EXAMPLE.com "),
            Ok("alex+me@example.com".into())
        );
        for email in [
            "",
            "a",
            "a@",
            "a@@example.com",
            "a@-example.com",
            "a..b@example.com",
            "a@exam ple.com",
            "a\n@example.com",
            "a@example..com",
        ] {
            assert!(normalize_email(email).is_err(), "{email}");
        }
    }
    #[test]
    fn password_policy_is_length_only_and_does_not_trim() {
        assert!(validate_password("a long phrase").is_ok());
        assert!(validate_password("short").is_err());
        assert!(validate_password(&"a".repeat(4097)).is_err());
    }
}
