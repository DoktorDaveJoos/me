//! Bounded local context interpretation. Raw content has no serialization/Debug API.
use me_core::CredentialKind;
use url::Url;
use zeroize::Zeroizing;

pub const SIGNALS: &[&str] = &[
    "login",
    "sign up",
    "register",
    "create account",
    "create an account",
    "sign in",
    "new password",
    "update password",
    "password",
    "change password",
    "recovery codes",
    "backup codes",
    "api",
    "token",
    "ssh",
    "private key",
    "public key",
    "wifi",
    "network",
    "database",
    "server",
];
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Create(CredentialKind),
    PasswordUpdate,
    RecoveryCodes,
    Unknown,
}
impl Intent {
    pub fn label(self) -> &'static str {
        match self {
            Self::Create(k) => k.label(),
            Self::PasswordUpdate => "Update a password",
            Self::RecoveryCodes => "Add recovery codes",
            Self::Unknown => "Choose an item type",
        }
    }
    pub fn kind(self) -> CredentialKind {
        match self {
            Self::Create(k) => k,
            _ => CredentialKind::Login,
        }
    }
}
pub struct Capture {
    pub source: String,
    pub registration: bool,
    pub email_expected: bool,
    pub password_present: bool,
    pub fill_target: Option<Zeroizing<String>>,
    pub website: String,
    pub intent: Intent,
    pub signals: Vec<&'static str>,
    pub values: Vec<(String, Zeroizing<String>)>,
    pub incomplete: bool,
    pub unassigned: Option<Zeroizing<String>>,
}
pub fn origin(value: &str) -> Option<String> {
    let u = Url::parse(value).ok()?;
    if !matches!(u.scheme(), "https" | "http") || u.host_str().is_none() {
        return None;
    }
    Some(u.origin().ascii_serialization())
}
fn field(label: &str) -> Option<&'static str> {
    match normalized_label(label).as_str() {
        "username" | "user name" | "email" | "email address" | "your email"
        | "your email address" | "e mail" | "e mail address" | "account" => Some("username"),
        "name" | "full name" | "your name" => Some("full_name"),
        "password" | "new password" | "create password" => Some("password"),
        "api key" | "access token" | "api token" | "token" => Some("token"),
        "private key" => Some("private_key"),
        "public key" => Some("public_key"),
        "recovery codes" | "backup codes" => Some("recovery_codes"),
        "network name" | "ssid" => Some("ssid"),
        "project" => Some("project"),
        "environment" => Some("environment"),
        "permissions" | "scopes" => Some("scopes"),
        "host" | "hostname" => Some("hostname"),
        "port" => Some("port"),
        _ => None,
    }
}
fn normalized_label(label: &str) -> String {
    label
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty() && *s != "required")
        .collect::<Vec<_>>()
        .join(" ")
}
/// Draft inference may use page labels/path; filling still requires the helper's verified target.
fn registration_evidence(data: &serde_json::Value) -> bool {
    let nodes = data["nodes"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let labels: Vec<_> = nodes
        .iter()
        .filter_map(|n| n["label"].as_str())
        .map(normalized_label)
        .collect();
    let keys: Vec<_> = nodes
        .iter()
        .filter_map(|n| {
            n["key"]
                .as_str()
                .or_else(|| n["label"].as_str().and_then(field))
        })
        .collect();
    let path = Url::parse(data["url"].as_str().unwrap_or(""))
        .ok()
        .map(|u| u.path().to_lowercase())
        .unwrap_or_default();
    let segments: Vec<_> = path.split('/').collect();
    let registration_path = segments.iter().any(|s| {
        [
            "register",
            "registration",
            "signup",
            "sign-up",
            "join",
            "create-account",
        ]
        .contains(s)
    });
    let sign_in_path = segments.iter().any(|s| {
        [
            "login",
            "signin",
            "sign-in",
            "reset",
            "recover",
            "recovery",
            "forgot-password",
        ]
        .contains(s)
    });
    let heading = nodes
        .iter()
        .filter(|n| n["role"] == "AXHeading")
        .filter_map(|n| n["text"].as_str().or(n["label"].as_str()))
        .map(normalized_label)
        .any(|s| {
            ["sign up", "register", "create account", "create an account"]
                .iter()
                .any(|cue| s == *cue || s.starts_with(&format!("{cue} ")))
        });
    !sign_in_path
        && (registration_path || heading || data["draft_registration"] == true)
        && keys.contains(&"username")
        && keys.contains(&"password")
        && !labels.iter().any(|s| {
            [
                "current password",
                "change password",
                "reset password",
                "update password",
            ]
            .contains(&s.as_str())
        })
}
impl Capture {
    pub fn from_text(source: String, text: &str, website: &str) -> Self {
        let mut limit = text.len().min(65536);
        while !text.is_char_boundary(limit) {
            limit -= 1;
        }
        let bounded = &text[..limit];
        // The only model-visible information consists of these fixed vocabulary signals.
        let lower = Zeroizing::new(text.chars().take(65536).collect::<String>().to_lowercase());
        let words = Zeroizing::new(
            lower
                .split(|c: char| !c.is_alphanumeric())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" "),
        );
        let signals = SIGNALS
            .iter()
            .copied()
            .filter(|signal| format!(" {} ", words.as_str()).contains(&format!(" {signal} ")))
            .collect::<Vec<_>>();
        let has = |signal| signals.contains(&signal);
        let intent = if has("recovery codes") || has("backup codes") {
            Intent::RecoveryCodes
        } else if has("change password") || has("new password") || has("update password") {
            Intent::PasswordUpdate
        } else if has("private key") || has("ssh") {
            Intent::Create(CredentialKind::Ssh)
        } else if has("api") || has("token") {
            Intent::Create(CredentialKind::Api)
        } else if has("wifi") || has("network") {
            Intent::Create(CredentialKind::Wifi)
        } else if has("database") || has("server") {
            Intent::Create(CredentialKind::Server)
        } else if has("login")
            || has("sign up")
            || has("sign in")
            || has("register")
            || has("create account")
            || has("create an account")
        {
            Intent::Create(CredentialKind::Login)
        } else {
            Intent::Unknown
        };
        let mut result = Self {
            source,
            registration: false,
            email_expected: true,
            password_present: false,
            fill_target: None,
            website: origin(website).unwrap_or_default(),
            intent,
            signals,
            values: Vec::new(),
            incomplete: text.len() > 65536,
            unassigned: None,
        };
        // Only explicit label:value pairs become values. Never guess account/secret from arbitrary prose.
        for line in text.lines().take(512) {
            if let Some((label, value)) = line.split_once(':') {
                if label.trim().eq_ignore_ascii_case("website")
                    || label.trim().eq_ignore_ascii_case("url")
                {
                    if result.website.is_empty() {
                        result.website = origin(value.trim()).unwrap_or_default();
                    }
                } else if let Some(key) = field(label) {
                    result.set(key, value.trim());
                }
            }
        }
        // Copy/paste may contain a complete private key. Preserve exact bytes including newlines.
        if let Some(start) = text.find("-----BEGIN OPENSSH PRIVATE KEY-----")
            && let Some(end) = text[start..].find("-----END OPENSSH PRIVATE KEY-----")
        {
            result.set(
                "private_key",
                &text[start..start + end + "-----END OPENSSH PRIVATE KEY-----".len()],
            );
            result.intent = Intent::Create(CredentialKind::Ssh);
        }
        {
            let code = regex::Regex::new(r"^[A-Za-z0-9]{4,6}(?:-[A-Za-z0-9]{4,6}){1,3}$")
                .expect("fixed expression");
            let codes = bounded
                .lines()
                .map(str::trim)
                .filter(|s| code.is_match(s))
                .take(100)
                .collect::<Vec<_>>();
            if codes.len() >= 2
                && codes.len() == bounded.lines().filter(|s| !s.trim().is_empty()).count()
            {
                result.intent = Intent::RecoveryCodes;
            }
            if result.intent == Intent::RecoveryCodes && !codes.is_empty() {
                result.set("recovery_codes", &codes.join("\n"));
            }
        }
        if result.values.is_empty() {
            result.unassigned = Some(Zeroizing::new(bounded.into()));
        }
        result
    }
    pub fn set(&mut self, key: &str, value: &str) {
        if value.is_empty()
            || value.len() > 65536
            || value.chars().all(|c| matches!(c, '•' | '●' | '*' | ' '))
        {
            return;
        }
        if let Some((_, old)) = self.values.iter_mut().find(|(k, _)| k == key) {
            *old = Zeroizing::new(value.into());
        } else {
            self.values.push((key.into(), Zeroizing::new(value.into())));
        }
    }
    /// Only recognized registration drafts receive a new password; sign-in and recovery do not.
    pub fn prepare_registration(&mut self, default_email: &str) -> Result<(), me_core::Error> {
        if !self.registration {
            return Ok(());
        }
        if !self
            .values
            .iter()
            .any(|(k, v)| k == "username" && !v.is_empty())
            && !default_email.is_empty()
        {
            self.set("username", default_email);
        }
        if !self.values.iter().any(|(k, _)| k == "password") {
            self.values
                .push(("password".into(), me_core::generate_credential_password()?));
        }
        if !self.values.iter().any(|(k, _)| k == "tags") {
            self.set("tags", "Web");
        }
        if !self.website.is_empty() && !self.values.iter().any(|(k, _)| k == "notes") {
            self.set(
                "notes",
                &format!(
                    "Account registration at {}. Review the website form before submitting.",
                    self.website
                ),
            );
        }
        Ok(())
    }
    pub fn from_window(raw: &[u8]) -> Result<Self, &'static str> {
        let parsed =
            PrivateJson(serde_json::from_slice(raw).map_err(|_| "Couldn't read this window.")?);
        let data = &parsed.0;
        if data["error"].as_str() == Some("permission") {
            return Err(
                "Allow ME. in System Settings → Privacy & Security → Accessibility, then try again. You can also paste instead.",
            );
        }
        if data["error"].is_string() {
            return Err("This window is unavailable. Choose another window or paste instead.");
        }
        let mut text = Zeroizing::new(String::new());
        for node in data["nodes"]
            .as_array()
            .ok_or("No readable content in this window.")?
        {
            if let Some(label) = node["label"].as_str() {
                text.push_str(label);
                text.push('\n');
            }
            if let Some(value) = node["text"].as_str() {
                text.push_str(value);
                text.push('\n');
            }
        }
        if let Some(ocr) = data["ocr"].as_str() {
            text.push_str(ocr);
        }
        let mut result = Self::from_text(
            data["source"].as_str().unwrap_or("Selected window").into(),
            &text,
            data["url"].as_str().unwrap_or(""),
        );
        result.unassigned = None;
        result.password_present = data["password_present"] == true;
        result.registration = (data["registration"] == true || registration_evidence(data))
            && !result.password_present
            && !result.website.is_empty();
        result.email_expected = data["nodes"].as_array().unwrap().iter().any(|n| {
            n["label"]
                .as_str()
                .is_some_and(|s| normalized_label(s).contains("mail"))
        });
        if result.registration {
            result.intent = Intent::Create(CredentialKind::Login);
            if !result.signals.contains(&"sign up") {
                result.signals.push("sign up");
            }
            if data["registration"] == true && data["fill_target"].is_object() {
                result.fill_target = Some(Zeroizing::new(data["fill_target"].to_string()));
            }
        }
        result.incomplete |= data["truncated"].as_bool().unwrap_or(false);
        for node in data["nodes"].as_array().unwrap() {
            if let (Some(key), Some(value)) = (
                node["key"]
                    .as_str()
                    .filter(|k| ["username", "password", "full_name"].contains(k))
                    .or_else(|| node["label"].as_str().and_then(field)),
                node["value"].as_str(),
            ) {
                result.set(key, value);
            }
        }
        Ok(result)
    }
}
struct PrivateJson(serde_json::Value);
impl Drop for PrivateJson {
    fn drop(&mut self) {
        fn wipe(v: &mut serde_json::Value) {
            match v {
                serde_json::Value::String(s) => zeroize::Zeroize::zeroize(s),
                serde_json::Value::Array(a) => a.iter_mut().for_each(wipe),
                serde_json::Value::Object(o) => o.values_mut().for_each(wipe),
                _ => {}
            }
        }
        wipe(&mut self.0);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registration_draft_does_not_depend_on_an_autofill_target() {
        let mut c = Capture::from_window(br#"{"source":"Chrome","nodes":[{"label":"Your email address *"},{"label":"Password (required)"},{"label":"Sign in"}],"url":"https://forge.example.test/register","registration":false,"truncated":true}"#).unwrap();
        c.prepare_registration("usual@example.test").unwrap();
        assert!(c.registration && c.fill_target.is_none());
        for key in ["username", "password", "tags", "notes"] {
            assert!(c.values.iter().any(|(k, v)| k == key && !v.is_empty()));
        }
        let before = c
            .values
            .iter()
            .find(|(k, _)| k == "password")
            .unwrap()
            .1
            .clone();
        c.prepare_registration("other@example.test").unwrap();
        assert_eq!(
            c.values
                .iter()
                .find(|(k, _)| k == "password")
                .unwrap()
                .1
                .as_str(),
            before.as_str()
        );
    }
    #[test]
    fn sign_in_and_reset_pages_with_registration_links_do_not_generate() {
        for path in ["login", "reset", "forgot-password"] {
            let raw = serde_json::json!({"source":"Chrome","nodes":[{"label":"Email"},{"label":"Password"},{"role":"AXLink","label":"Create an account"}],"url":format!("https://example.test/{path}")});
            let mut c = Capture::from_window(raw.to_string().as_bytes()).unwrap();
            c.prepare_registration("usual@example.test").unwrap();
            assert!(!c.registration);
            assert!(!c.values.iter().any(|(k, _)| k == "password"));
        }
    }
    #[test]
    fn registration_drafts_use_origin_and_generate_locally_without_changing_typed_email() {
        let mut c = Capture::from_window(br#"{"source":"Chrome","nodes":[{"label":"Email","value":"chosen@example.test"},{"label":"New password"}],"url":"https://forge.laravel.com/register?private=value#fragment","registration":true,"fill_target":{"page":"https://forge.laravel.com/register"}}"#).unwrap();
        c.prepare_registration("default@example.test").unwrap();
        assert!(c.intent == Intent::Create(CredentialKind::Login));
        assert_eq!(c.website, "https://forge.laravel.com");
        assert_eq!(
            c.values
                .iter()
                .find(|(k, _)| k == "username")
                .unwrap()
                .1
                .as_str(),
            "chosen@example.test"
        );
        assert_eq!(
            c.values
                .iter()
                .find(|(k, _)| k == "password")
                .unwrap()
                .1
                .len(),
            20
        );
        assert!(c.fill_target.is_some());
        assert!(c.signals.iter().all(|s| SIGNALS.contains(s)));
    }
    #[test]
    fn plain_login_does_not_generate_a_replacement_password() {
        let mut c = Capture::from_window(br#"{"source":"Chrome","nodes":[{"label":"Sign in"},{"label":"Email"}],"url":"https://example.test/login"}"#).unwrap();
        c.prepare_registration("default@example.test").unwrap();
        assert!(!c.registration);
        assert!(c.values.is_empty());
        assert!(c.fill_target.is_none());
    }
    #[test]
    fn default_email_is_used_only_for_an_empty_registration_and_existing_passwords_are_not_replaced()
     {
        let mut c = Capture::from_window(br#"{"source":"Chrome","nodes":[{"label":"Email"}],"url":"https://example.test/register","registration":true}"#).unwrap();
        c.prepare_registration("default@example.test").unwrap();
        assert_eq!(
            c.values
                .iter()
                .find(|(k, _)| k == "username")
                .unwrap()
                .1
                .as_str(),
            "default@example.test"
        );
        let mut occupied = Capture::from_window(br#"{"source":"Chrome","nodes":[{"label":"Sign up"}],"url":"https://example.test/register","registration":true,"password_present":true,"fill_target":{"page":"https://example.test/register"}}"#).unwrap();
        occupied
            .prepare_registration("default@example.test")
            .unwrap();
        assert!(!occupied.registration);
        assert!(!occupied.values.iter().any(|(k, _)| k == "password"));
        assert!(occupied.fill_target.is_none());
    }
    #[test]
    fn detects_intent_without_sending_values_or_urls_to_ai() {
        let c = Capture::from_text(
            "Paste".into(),
            "Recovery codes\nABCD-EFGH\nIJKL-MNOP\nemail: private@example.test",
            "https://example.test/reset?token=SECRET#private",
        );
        assert!(c.intent == Intent::RecoveryCodes);
        assert_eq!(c.website, "https://example.test");
        assert_eq!(
            c.values
                .iter()
                .find(|(k, _)| k == "recovery_codes")
                .unwrap()
                .1
                .as_str(),
            "ABCD-EFGH\nIJKL-MNOP"
        );
        assert!(c.signals.iter().all(|s| SIGNALS.contains(s)));
        assert!(!c.signals.join(" ").contains("private"));
    }
    #[test]
    fn ignores_masked_values_and_does_not_guess_random_passwords() {
        let c = Capture::from_text(
            "Paste".into(),
            "password: ••••••••\nrandom unlabelled text",
            "",
        );
        assert!(c.values.is_empty());
        assert!(c.intent == Intent::Unknown);
    }
    #[test]
    fn bare_codes_are_recognized_but_an_unlabelled_secret_requires_a_type_choice() {
        let c = Capture::from_text("Clipboard".into(), "ABCD-EFGH\nIJKL-MNOP", "");
        assert!(c.intent == Intent::RecoveryCodes);
        assert_eq!(c.values[0].0, "recovery_codes");
        let c = Capture::from_text("Clipboard".into(), "  SYNTHETIC-token-without-label  ", "");
        assert!(c.intent == Intent::Create(CredentialKind::Api));
        let c = Capture::from_text("Clipboard".into(), "  XYZ123  ", "");
        assert!(c.intent == Intent::Unknown);
        assert_eq!(c.unassigned.unwrap().as_str(), "  XYZ123  ");
    }
    #[test]
    fn structured_values_preserve_spaces() {
        let c=Capture::from_window(br#"{"source":"Synthetic","nodes":[{"label":"New password","value":"  TEST  "}],"url":"https://example.test/path"}"#).unwrap();
        assert_eq!(c.values[0].1.as_str(), "  TEST  ");
    }
}
