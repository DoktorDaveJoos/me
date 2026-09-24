//! Suggestions stay inside the unlocked vault. No plaintext index, logging or AI export.
use crate::{LoginPresentation, Result, Vault};
use std::collections::{BTreeMap, BTreeSet};
use zeroize::Zeroizing;

pub struct LoginIdentity {
    pub value: Zeroizing<String>,
    pub uses: usize,
    pub websites: Vec<String>,
}
#[derive(Default)]
pub struct CredentialSuggestions {
    pub identities: Vec<LoginIdentity>,
    pub tags: Vec<String>,
}
impl CredentialSuggestions {
    pub fn preferred_identity(&self, website: &str, email_only: bool) -> Option<&str> {
        let host = |s: &str| {
            url::Url::parse(s)
                .ok()
                .and_then(|u| u.host_str().map(str::to_owned))
        };
        let target = host(website);
        self.identities
            .iter()
            .filter(|i| !email_only || i.value.contains('@'))
            .max_by_key(|i| {
                (
                    target.is_some() && i.websites.iter().any(|w| host(w) == target),
                    i.uses,
                )
            })
            .map(|i| i.value.as_str())
    }
}
impl Vault {
    pub fn credential_suggestions(&self) -> Result<CredentialSuggestions> {
        let mut identities: BTreeMap<String, (usize, BTreeSet<String>)> = BTreeMap::new();
        let mut tags = BTreeSet::new();
        for item in self
            .logins()?
            .into_iter()
            .filter(|i| !i.archived && i.version == i.versions)
        {
            let details = self.login_details(item.id)?;
            let mut account_values = BTreeSet::new();
            for field in details.fields {
                if field.presentation == LoginPresentation::Tags {
                    tags.extend(
                        field
                            .value
                            .lines()
                            .map(str::trim)
                            .filter(|v| !v.is_empty())
                            .map(str::to_owned),
                    );
                }
                let label = field.label.to_lowercase();
                if item.category == "001"
                    && !field.concealed
                    && (field.key == "username"
                        || [
                            "username",
                            "username / email",
                            "email",
                            "email address",
                            "e-mail",
                            "user name",
                        ]
                        .contains(&label.as_str()))
                {
                    let value = field.value.trim();
                    if !value.is_empty()
                        && value.len() <= 320
                        && !value.chars().any(char::is_control)
                    {
                        account_values.insert(value.to_owned());
                    }
                }
            }
            for value in account_values {
                let (uses, websites) = identities.entry(value).or_default();
                *uses += 1;
                if !item.website.is_empty() {
                    websites.insert(item.website.clone());
                }
            }
        }
        let mut identities: Vec<_> = identities
            .into_iter()
            .map(|(value, (uses, websites))| LoginIdentity {
                value: Zeroizing::new(value),
                uses,
                websites: websites.into_iter().collect(),
            })
            .collect();
        identities.sort_by(|a, b| b.uses.cmp(&a.uses).then_with(|| a.value.cmp(&b.value)));
        Ok(CredentialSuggestions {
            identities,
            tags: tags.into_iter().collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CredentialKind, LoginUpdate, OnePasswordImport};
    fn add(v: &mut Vault, email: &str, website: &str) -> u64 {
        v.create_credential(
            CredentialKind::Login,
            LoginUpdate {
                revision: 0,
                title: "Synthetic".into(),
                favorite: false,
                fields: [
                    ("username", email),
                    ("website", website),
                    ("password", "NEVER-SUGGEST-SECRET"),
                    ("notes", "NEVER-SUGGEST-NOTE"),
                    ("tags", "Work\nDeveloper"),
                ]
                .into_iter()
                .map(|(k, v)| (k.into(), Zeroizing::new(v.into())))
                .collect(),
            },
        )
        .unwrap()
    }
    #[test]
    fn imported_and_native_values_are_deduplicated_ranked_and_updated() {
        let tmp = tempfile::tempdir().unwrap();
        let mut v = Vault::create(&tmp.path().join("vault"), "synthetic-passphrase").unwrap();
        v.import_onepassword(
            &OnePasswordImport::read(
                &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/synthetic-logins.1pux"),
            )
            .unwrap(),
        )
        .unwrap();
        let imported = v.credential_suggestions().unwrap().identities.len();
        assert!(imported > 0);
        assert!(
            !v.credential_suggestions()
                .unwrap()
                .identities
                .iter()
                .any(|i| [
                    "alex7@example.com",
                    "alex20@example.com",
                    "alex21@example.com"
                ]
                .contains(&i.value.as_str()))
        );
        add(&mut v, "frequent@example.test", "https://a.example.test");
        add(&mut v, "frequent@example.test", "https://b.example.test");
        let id = add(
            &mut v,
            "specific@example.test",
            "https://forge.example.test/register",
        );
        let suggestions = v.credential_suggestions().unwrap();
        assert_eq!(suggestions.identities.len(), imported + 2);
        assert_eq!(
            suggestions.preferred_identity("https://forge.example.test/", true),
            Some("specific@example.test")
        );
        assert_eq!(
            suggestions.preferred_identity("https://new.example.test", true),
            Some("frequent@example.test")
        );
        assert!(suggestions.tags.contains(&"Developer".into()));
        assert!(
            !suggestions
                .identities
                .iter()
                .any(|i| i.value.contains("NEVER-SUGGEST"))
        );
        let details = v.login_details(id).unwrap();
        v.update_login(
            id,
            LoginUpdate {
                revision: details.revision,
                title: "Synthetic".into(),
                favorite: false,
                fields: vec![(
                    "username".into(),
                    Zeroizing::new("updated@example.test".into()),
                )],
            },
        )
        .unwrap();
        assert_eq!(
            v.credential_suggestions()
                .unwrap()
                .preferred_identity("https://forge.example.test", true),
            Some("updated@example.test")
        );
    }
}
