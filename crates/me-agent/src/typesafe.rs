//! Narrow typed decisions only. No generated identifiers, no automatic retries,
//! fixed HTTPS destination, bounded response body, and no payload/header logging.
use curl::easy::{Easy, List};
use me_core::{ImportErrorKind as Kind, ImportFailure, ImportProvider};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use zeroize::Zeroizing;

/// Pinned so tuned thresholds keep their meaning; every answer records the version
/// that produced it. Move to a new version deliberately, with its thresholds.
pub const MODEL: &str = crate::typesafe_questions::MODEL;
/// Attempts for rate-limit (429) and overload (529) responses, honoring `retry-after`.
const RATE_LIMIT_ATTEMPTS: u32 = 3;
const MAX_BACKOFF: Duration = Duration::from_secs(20);
pub type Result<T> = std::result::Result<T, ImportFailure>;
fn problem(kind: Kind, message: &str) -> ImportFailure {
    ImportFailure::new(ImportProvider::TypeSafe, kind, message)
}
fn invalid() -> ImportFailure {
    problem(
        Kind::InvalidOutput,
        "TypeSafe returned an invalid decision. Saved steps are kept; retry explicitly.",
    )
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DecisionResponse {
    pub answers: Value,
    /// Versioned model that answered, as reported by the API.
    #[serde(default)]
    pub model: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}
pub trait Decisions {
    fn evaluate(
        &mut self,
        state: Value,
        questions: Value,
        cancel: &AtomicBool,
    ) -> Result<DecisionResponse>;
}
pub struct TypeSafe {
    key: Zeroizing<String>,
}
impl TypeSafe {
    pub fn configured() -> Result<Self> {
        let key=std::env::var("TYPESAFE_API_KEY").ok().filter(|s| !s.trim().is_empty()).or_else(|| {
            let path=std::env::var_os("ME_TYPESAFE_ENV_FILE").map(std::path::PathBuf::from)
                .unwrap_or_else(||Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.env"));
            // Read at runtime only. Never include a key or .env file in the binary.
            read_env_key(&path)
        }).ok_or_else(||problem(Kind::Authentication,"TypeSafe is not configured. Add TYPESAFE_API_KEY to your private .env file, then resume imports."))?;
        if key.len() > 512 || key.chars().any(char::is_whitespace) {
            return Err(problem(
                Kind::Authentication,
                "The TypeSafe key has an invalid format. Update the private configuration.",
            ));
        }
        Ok(Self {
            key: Zeroizing::new(key),
        })
    }
}
#[cfg(not(test))]
#[derive(Default)]
pub(crate) struct LazyDecisions(Option<TypeSafe>);
#[cfg(not(test))]
impl Decisions for LazyDecisions {
    fn evaluate(
        &mut self,
        state: Value,
        questions: Value,
        cancel: &AtomicBool,
    ) -> Result<DecisionResponse> {
        if self.0.is_none() {
            self.0 = Some(TypeSafe::configured()?);
        }
        self.0
            .as_mut()
            .ok_or_else(invalid)?
            .evaluate(state, questions, cancel)
    }
}
fn read_env_key(path: &Path) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file()
        || meta.file_type().is_symlink()
        || meta.len() > 64 * 1024
        || meta.permissions().mode() & 0o077 != 0
    {
        return None;
    }
    let text = Zeroizing::new(std::fs::read_to_string(path).ok()?);
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("TYPESAFE_API_KEY="))
        .next_back()
        .map(|key| key.trim().trim_matches(['\'', '"']).to_owned())
        .filter(|s| !s.is_empty())
}
impl Decisions for TypeSafe {
    fn evaluate(
        &mut self,
        state: Value,
        questions: Value,
        cancel: &AtomicBool,
    ) -> Result<DecisionResponse> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            match self.attempt(&state, &questions, cancel) {
                Err(Attempt::Busy(wait)) if attempt < RATE_LIMIT_ATTEMPTS => {
                    let wait = wait
                        .unwrap_or(Duration::from_secs(1 << attempt))
                        .min(MAX_BACKOFF);
                    let until = std::time::Instant::now() + wait;
                    while std::time::Instant::now() < until {
                        if cancel.load(Ordering::SeqCst) {
                            return Err(problem(
                                Kind::Cancelled,
                                "Analysis stopped. Saved steps are kept.",
                            ));
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
                Err(Attempt::Busy(_)) => {
                    return Err(problem(
                        Kind::RateLimit,
                        "TypeSafe is busy or rate limited. Imports are paused; resume shortly.",
                    ));
                }
                Err(Attempt::Failed(failure)) => return Err(failure),
                Ok(response) => return Ok(response),
            }
        }
    }
}
enum Attempt {
    /// 429 or 529, with the server's `retry-after` when given.
    Busy(Option<Duration>),
    Failed(ImportFailure),
}
impl From<ImportFailure> for Attempt {
    fn from(f: ImportFailure) -> Self {
        Self::Failed(f)
    }
}
impl TypeSafe {
    fn attempt(
        &mut self,
        state: &Value,
        questions: &Value,
        cancel: &AtomicBool,
    ) -> std::result::Result<DecisionResponse, Attempt> {
        if cancel.load(Ordering::SeqCst) {
            return Err(problem(Kind::Cancelled, "Analysis stopped. Saved steps are kept.").into());
        }
        let body = Zeroizing::new(
            serde_json::to_vec(&json!({"model":MODEL,"state":state,"questions":questions}))
                .map_err(|_| invalid())?,
        );
        if body.len() > 160_000 {
            return Err(problem(
                Kind::Unprocessable,
                "This section exceeds the TypeSafe request limit. Split the document.",
            )
            .into());
        }
        let transport = |_: curl::Error| {
            problem(
                Kind::Connection,
                "Could not connect to TypeSafe. Check the connection, then resume.",
            )
        };
        let mut easy = Easy::new();
        easy.url("https://api.typesafe.ai/v1/systemone")
            .map_err(transport)?;
        easy.follow_location(false).map_err(transport)?;
        easy.connect_timeout(Duration::from_secs(10))
            .map_err(transport)?;
        easy.timeout(Duration::from_secs(45)).map_err(transport)?;
        easy.progress(true).map_err(transport)?;
        easy.post_fields_copy(&body).map_err(transport)?;
        let mut headers = List::new();
        headers
            .append(&format!("Authorization: Bearer {}", *self.key))
            .map_err(transport)?;
        headers
            .append("Content-Type: application/json")
            .map_err(transport)?;
        easy.http_headers(headers).map_err(transport)?;
        let mut bytes = Zeroizing::new(Vec::new());
        let mut oversized = false;
        let mut retry_after = None;
        let outcome = {
            let mut transfer = easy.transfer();
            transfer
                .header_function(|line| {
                    if let Ok(line) = std::str::from_utf8(line)
                        && let Some((name, value)) = line.split_once(':')
                        && name.trim().eq_ignore_ascii_case("retry-after")
                    {
                        retry_after = value.trim().parse::<u64>().ok().map(Duration::from_secs);
                    }
                    true
                })
                .map_err(transport)?;
            transfer
                .progress_function(|_, _, _, _| !cancel.load(Ordering::SeqCst))
                .map_err(transport)?;
            transfer
                .write_function(|part| {
                    if bytes.len() + part.len() > 256 * 1024 {
                        oversized = true;
                        return Ok(0);
                    }
                    bytes.extend_from_slice(part);
                    Ok(part.len())
                })
                .map_err(transport)?;
            transfer.perform()
        };
        if cancel.load(Ordering::SeqCst) {
            return Err(problem(Kind::Cancelled, "Analysis stopped. Saved steps are kept.").into());
        }
        if oversized {
            return Err(invalid().into());
        }
        if let Err(e) = outcome {
            return Err(if e.is_operation_timedout() {
                problem(
                    Kind::Timeout,
                    "TypeSafe timed out. No automatic retry was made; resume to reuse saved steps.",
                )
            } else {
                transport(e)
            }
            .into());
        }
        let status = easy.response_code().map_err(transport)?;
        match status {
            200..=299 => {}
            401 | 403 => {
                return Err(problem(
                    Kind::Authentication,
                    "TypeSafe rejected the API key. Update the private configuration and resume the queue.",
                ).into());
            }
            402 => {
                return Err(problem(
                    Kind::Quota,
                    "TypeSafe credits are exhausted. Imports are paused; check your account before resuming.",
                ).into());
            }
            429 | 529 => return Err(Attempt::Busy(retry_after)),
            408 | 504 => {
                return Err(problem(
                    Kind::Timeout,
                    "TypeSafe timed out. Saved steps are kept. Resume when the service is available.",
                ).into());
            }
            500..=599 => {
                return Err(problem(
                    Kind::Connection,
                    "TypeSafe is temporarily unavailable. Saved steps are kept; no automatic retry was made.",
                ).into());
            }
            _ => {
                return Err(problem(
                    Kind::InvalidOutput,
                    "TypeSafe declined this request. Check the integration before retrying.",
                )
                .into());
            }
        }
        let response: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        validate_answers(questions, &response["answers"])?;
        Ok(DecisionResponse {
            answers: response["answers"].clone(),
            model: response["model"].as_str().map(str::to_owned),
            input_tokens: response
                .pointer("/usage/input_tokens")
                .and_then(Value::as_u64),
            output_tokens: response
                .pointer("/usage/output_tokens")
                .and_then(Value::as_u64),
        })
    }
}
fn probability(value: &Value) -> Result<f64> {
    value
        .as_f64()
        .filter(|n| n.is_finite() && (0.0..=1.0).contains(n))
        .ok_or_else(invalid)
}
pub(crate) fn validate_answers(questions: &Value, answers: &Value) -> Result<()> {
    let questions = questions.as_object().ok_or_else(invalid)?;
    let answers = answers.as_object().ok_or_else(invalid)?;
    for (id, q) in questions {
        let answer = answers.get(id).ok_or_else(invalid)?;
        if answer["type"] != q["type"] {
            return Err(invalid());
        }
        match q["type"].as_str() {
            Some("noul") => {
                probability(&answer["noul"])?;
            }
            Some("choice") => {
                let options = q["criteria"].as_object().ok_or_else(invalid)?;
                let choice = answer["choice"].as_str().ok_or_else(invalid)?;
                if !options.contains_key(choice) {
                    return Err(invalid());
                }
                probability(&answer["confidence"])?;
                let probs = answer["probabilities"].as_object().ok_or_else(invalid)?;
                if probs.len() != options.len() {
                    return Err(invalid());
                }
                let sum = options.keys().try_fold(0., |sum, k| {
                    probability(probs.get(k).ok_or_else(invalid)?).map(|p| sum + p)
                })?;
                if (sum - 1.).abs() > 0.01
                    || probs.values().any(|p| {
                        p.as_f64().unwrap_or(0.) > probs[choice].as_f64().unwrap_or(0.) + 0.000001
                    })
                {
                    return Err(invalid());
                }
            }
            Some("score") => {
                let levels = q["criteria"].as_array().ok_or_else(invalid)?.len();
                let score = answer["score"].as_f64().ok_or_else(invalid)?;
                if levels < 2 || !(0.0..=(levels - 1) as f64 + 1e-6).contains(&score) {
                    return Err(invalid());
                }
                probability(&answer["confidence"])?;
                let probs = answer["probabilities"].as_object().ok_or_else(invalid)?;
                if probs.len() != levels {
                    return Err(invalid());
                }
                let sum = (0..levels).try_fold(0., |sum, level| {
                    probability(probs.get(&level.to_string()).ok_or_else(invalid)?).map(|p| sum + p)
                })?;
                if (sum - 1.).abs() > 0.01 {
                    return Err(invalid());
                }
            }
            _ => return Err(invalid()),
        }
    }
    Ok(())
}
pub(crate) use crate::typesafe_questions::{profile_questions, verification_questions};
pub(crate) fn needs_audit(answers: &Value) -> Result<bool> {
    use crate::typesafe_questions::AUDIT_TRIGGER;
    Ok(probability(&answers["missing"]["noul"])? >= AUDIT_TRIGGER
        || probability(&answers["misassigned"]["noul"])? >= AUDIT_TRIGGER)
}
#[cfg(test)]
pub(crate) struct FakeDecisions;
#[cfg(test)]
impl Decisions for FakeDecisions {
    fn evaluate(&mut self, _: Value, questions: Value, _: &AtomicBool) -> Result<DecisionResponse> {
        let answers=questions.as_object().unwrap().iter().map(|(id,q)|{
            let v=if q["type"]=="choice" {let keys=q["criteria"].as_object().unwrap();json!({"type":"choice","choice":"other","confidence":1.,"probabilities":keys.keys().map(|k|(k.clone(),if k=="other"{1.}else{0.})).collect::<std::collections::BTreeMap<_,_>>()})} else {json!({"type":"noul","noul":if id=="readable"{1.}else{0.}})};
            (id.clone(),v)
        }).collect::<serde_json::Map<_,_>>();
        Ok(DecisionResponse {
            answers: Value::Object(answers),
            model: Some("jev-test".into()),
            input_tokens: Some(50),
            output_tokens: Some(10),
        })
    }
}
/// Credential classification receives only fixed local vocabulary, never captured text or values.
pub fn credential_intent(signals: &[&str], cancel: &AtomicBool) -> Result<Option<String>> {
    const ALLOWED: &[&str] = &[
        "login",
        "sign up",
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
    if signals.iter().any(|s| !ALLOWED.contains(s)) {
        return Err(invalid());
    }
    let questions = crate::typesafe_questions::credential_intent_questions();
    let response =
        TypeSafe::configured()?.evaluate(json!({"label_concepts":signals}), questions, cancel)?;
    let a = &response.answers["intent"];
    if a["confidence"].as_f64().unwrap_or(0.)
        < crate::typesafe_questions::CREDENTIAL_INTENT_CONFIDENT
        || a["choice"] == "unknown"
    {
        return Ok(None);
    }
    Ok(a["choice"].as_str().map(str::to_owned))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credential_decisions_reject_free_text_before_configuration_or_network() {
        assert!(credential_intent(&["private@example.test"], &AtomicBool::new(false)).is_err());
        assert!(credential_intent(&["SYNTHETIC-PRIVATE-KEY"], &AtomicBool::new(false)).is_err());
    }
    #[test]
    #[ignore = "One synthetic credential decision; requires private TypeSafe configuration"]
    fn live_credential_intent() {
        assert_eq!(
            credential_intent(&["api", "token"], &AtomicBool::new(false))
                .unwrap()
                .as_deref(),
            Some("api")
        );
    }
    #[test]
    fn rejects_missing_malformed_and_out_of_range_decisions() {
        let q = verification_questions();
        for a in [
            json!({}),
            json!({"missing":{"type":"noul","noul":1.2},"misassigned":{"type":"noul","noul":0}}),
            json!({"missing":{"type":"noul","noul":0}}),
        ] {
            assert!(validate_answers(&q, &a).is_err());
        }
        let valid =
            json!({"missing":{"type":"noul","noul":0.1},"misassigned":{"type":"noul","noul":0.5}});
        validate_answers(&q, &valid).unwrap();
        assert!(needs_audit(&valid).unwrap());
    }
    #[test]
    fn uncertain_semantics_trigger_one_audit_not_automatic_acceptance() {
        assert!(needs_audit(&json!({"missing":{"noul":0.5},"misassigned":{"noul":0.}})).unwrap());
        assert!(
            !needs_audit(&json!({"missing":{"noul":0.01},"misassigned":{"noul":0.01}})).unwrap()
        );
    }
    #[test]
    #[ignore = "One live TypeSafe request using synthetic text; needs private API configuration"]
    fn live_synthetic_decisions_match_the_documented_contract() {
        let mut client = TypeSafe::configured().unwrap();
        let response = client
            .evaluate(
                json!({"text":"SYNTHETIC Invoice. Reference TEST-42. Amount due EUR 12.00."}),
                profile_questions(),
                &AtomicBool::new(false),
            )
            .unwrap();
        assert_eq!(response.answers["document_kind"]["choice"], "invoice");
        assert!(response.input_tokens.is_some());
        assert!(response.output_tokens.is_some());
        println!(
            "Synthetic TypeSafe check passed; reported input/output tokens: {}/{}",
            response.input_tokens.unwrap(),
            response.output_tokens.unwrap()
        );
    }
}
