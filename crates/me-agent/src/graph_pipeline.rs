//! Import stages that ask TypeSafe typed questions: Classify (one request per
//! document) and the reduce-side checks (value equivalence, organization
//! alignment, on-demand search). Reader facts are verified in `fact_verification`.
//! Code owns control flow, dates and arithmetic; TypeSafe answers narrow semantic
//! questions.
use crate::typesafe::{Decisions, Result};
use crate::typesafe_questions as q;
use me_core::{
    Candidate, CandidateKind, CandidateValue, DocType, IdentityAnchors, ImportErrorKind as Kind,
    ImportFailure, ImportProvider, MergeCandidate, SourceSegment, ValueConflict,
    doc_types::{self, REGISTRY_VERSION},
    normalize_name,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::sync::atomic::AtomicBool;

pub const CLASSIFY_STAGE: &str = "classify";
/// Bytes of the first and the last page sent for classification.
const CLASSIFY_PAGE_BYTES: usize = 6_000;
/// Bytes of document excerpts per on-demand search request.
const SEARCH_STATE_BYTES: usize = 20_000;

pub(crate) fn invalid() -> ImportFailure {
    ImportFailure::new(
        ImportProvider::TypeSafe,
        Kind::InvalidOutput,
        "TypeSafe returned an invalid decision. Saved steps are kept; retry explicitly.",
    )
}

/// The normalized text of one document.
#[derive(Clone, Copy)]
pub struct Document<'a> {
    pub title: &'a str,
    pub segments: &'a [SourceSegment<'a>],
}

/// One organization pair judged by the alignment Score.
#[derive(Clone, Debug, PartialEq)]
pub struct Alignment {
    pub a: String,
    pub b: String,
    /// Score position normalized to 0..1.
    pub score: f64,
    /// Show the user a merge proposal; never an automatic merge.
    pub propose: bool,
}

/// Usage of one stage, for batch accounting and audit.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StageUsage {
    pub requests: u32,
    pub input_tokens: u64,
    pub models: Vec<String>,
}
impl StageUsage {
    pub(crate) fn add(&mut self, r: &crate::typesafe::DecisionResponse) {
        self.requests += 1;
        self.input_tokens += r.input_tokens.unwrap_or(0);
        if let Some(m) = &r.model
            && !self.models.contains(m)
        {
            self.models.push(m.clone());
        }
    }
}

/// Cache version of Classify: registry, model and the anchors it compared against.
pub fn classify_version(anchors: &IdentityAnchors) -> String {
    let names: Vec<String> = anchors
        .names()
        .into_iter()
        .map(|(e, n)| format!("{e}:{n}"))
        .collect();
    format!(
        "classify-v1|{REGISTRY_VERSION}|{}|{}",
        q::MODEL,
        fingerprint(&names.join("|"))
    )
}
fn fingerprint(s: &str) -> String {
    // Stable, non-cryptographic cache discriminator (FNV-1a); never a secret.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Classification {
    pub family: String,
    pub family_confidence: f64,
    pub doc_type: Option<String>,
    pub type_confidence: Option<f64>,
    /// Anchor entity the document is about.
    pub subject: Option<String>,
    pub subject_confidence: f64,
    /// A non-anchor person named as the subject (counts towards a household proposal).
    pub subject_name: Option<String>,
    pub readable: f64,
    pub mixed: f64,
    pub usage: StageUsage,
}
impl Classification {
    /// The eager registry type whose values fill the profile, if classification
    /// was confident enough.
    pub fn eager_type(&self) -> Option<&'static DocType> {
        self.doc_type
            .as_deref()
            .and_then(doc_types::doc_type)
            .filter(|t| t.tier == me_core::Tier::Eager)
    }
}

fn prefix(text: &str, max: usize) -> &str {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
fn suffix(text: &str, max: usize) -> &str {
    let mut start = text.len().saturating_sub(max);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}
fn answer<'a>(answers: &'a Value, key: &str) -> Result<&'a Value> {
    answers.get(key).ok_or_else(invalid)
}
fn chosen(a: &Value) -> Result<(&str, f64)> {
    Ok((
        a["choice"].as_str().ok_or_else(invalid)?,
        a["confidence"].as_f64().ok_or_else(invalid)?,
    ))
}
fn noul(a: &Value) -> Result<f64> {
    a["noul"].as_f64().ok_or_else(invalid)
}

/// Anchor whose name matches a machine-readable zone exactly (order-insensitive).
fn mrz_anchor(
    candidates: &[Candidate],
    anchors: &IdentityAnchors,
) -> Option<(Option<String>, String)> {
    let mrz = candidates.iter().find_map(|c| match &c.value {
        CandidateValue::Mrz(m) if m.check_digits_valid => Some(m),
        _ => None,
    })?;
    let printed = format!("{} {}", mrz.given_names, mrz.surname);
    let key = |s: &str| {
        let mut words: Vec<String> = normalize_name(s).split(' ').map(str::to_owned).collect();
        words.sort();
        words.join(" ")
    };
    let wanted = key(&printed);
    let entity = anchors
        .names()
        .into_iter()
        .find(|(_, name)| key(name) == wanted)
        .map(|(e, _)| e);
    Some((entity, printed.trim().to_owned()))
}

/// Classify one document with a single fan-out request.
pub fn classify(
    doc: Document<'_>,
    anchors: &IdentityAnchors,
    candidates: &[Candidate],
    decisions: &mut impl Decisions,
    cancel: &AtomicBool,
) -> Result<Classification> {
    let (title, segments) = (doc.title, doc.segments);
    let first = segments.first().map_or("", |s| s.text);
    let last = if segments.len() > 1 {
        segments.last().map_or("", |s| s.text)
    } else {
        ""
    };
    let (first, last) = if last.is_empty() && first.len() > CLASSIFY_PAGE_BYTES {
        (
            prefix(first, CLASSIFY_PAGE_BYTES),
            suffix(first, CLASSIFY_PAGE_BYTES / 2),
        )
    } else {
        (
            prefix(first, CLASSIFY_PAGE_BYTES),
            suffix(last, CLASSIFY_PAGE_BYTES),
        )
    };
    let anchor_names: Vec<String> = anchors
        .names()
        .into_iter()
        .map(|(_, n)| normalize_name(&n))
        .collect();
    let mut found = Vec::new();
    for c in candidates
        .iter()
        .filter(|c| c.kind == CandidateKind::PersonName)
    {
        let name = c.text.trim().to_owned();
        let key = normalize_name(&name);
        if !key.is_empty()
            && !anchor_names.contains(&key)
            && !found.iter().any(|f: &String| normalize_name(f) == key)
        {
            found.push(name);
        }
    }
    let subjects = q::subject_options(anchors, &found);
    let mut document = json!({"file_name":title,"first_page":first});
    if !last.is_empty() {
        document["last_page"] = json!(last);
    }
    let response = decisions.evaluate(
        json!({"document":document}),
        q::classify_questions(&subjects),
        cancel,
    )?;
    let mut usage = StageUsage::default();
    usage.add(&response);
    let a = &response.answers;
    let (family, family_confidence) = chosen(answer(a, "family")?)?;
    let family = if family_confidence < q::FAMILY_FLOOR {
        "other"
    } else {
        family
    };
    let readable = noul(answer(a, "readable")?)?;
    let mixed = noul(answer(a, "mixed")?)?;
    let (mut doc_type, mut type_confidence) = (None, None);
    if let Some(t) = a.get(format!("type_{family}")) {
        let (choice, confidence) = chosen(t)?;
        type_confidence = Some(confidence);
        // Parent fallback: an uncertain type keeps only the family.
        if choice != "other"
            && confidence >= q::TYPE_CONFIDENT
            && readable >= q::READABLE_FLOOR
            && mixed < q::VERIFY_FIRE
        {
            doc_type = Some(choice.to_owned());
        }
    }
    let (mut subject, mut subject_confidence, mut subject_name) = (None, 0., None);
    let about = noul(answer(a, "about_person")?)?;
    let (who, confidence) = chosen(answer(a, "subject")?)?;
    if about >= q::ABOUT_PERSON && confidence >= q::SUBJECT_CONFIDENT {
        if let Some((_, entity)) = subjects.anchors.iter().find(|(k, _)| k == who) {
            subject = Some(entity.clone());
            subject_confidence = confidence;
        } else if let Some((_, name)) = subjects.named.iter().find(|(k, _)| k == who) {
            subject_name = Some(name.clone());
        }
    }
    // A valid machine-readable zone names its holder; that beats a judgment.
    if let Some((entity, printed)) = mrz_anchor(candidates, anchors) {
        match entity {
            Some(e) => {
                subject = Some(e);
                subject_confidence = 1.;
                subject_name = None;
            }
            None => {
                subject = None;
                subject_confidence = 0.;
                subject_name = Some(printed);
            }
        }
    }
    Ok(Classification {
        family: family.to_owned(),
        family_confidence,
        doc_type,
        type_confidence,
        subject,
        subject_confidence,
        subject_name,
        readable,
        mixed,
        usage,
    })
}

/// Reduce: which flagged value conflicts are only different spellings.
/// Returns (keep, duplicate) pairs; the duplicate is the second value.
pub fn judge_equivalence(
    conflicts: &[ValueConflict],
    decisions: &mut impl Decisions,
    cancel: &AtomicBool,
) -> Result<(Vec<(String, String)>, StageUsage)> {
    let mut usage = StageUsage::default();
    let mut out = Vec::new();
    for chunk in conflicts.chunks(20) {
        let pairs: Map<String, Value> = chunk
            .iter()
            .enumerate()
            .map(|(i, c)| {
                (
                    format!("p{i}"),
                    json!({"property":c.label,"first":c.first_value,"second":c.second_value}),
                )
            })
            .collect();
        let response = decisions.evaluate(
            json!({"pairs":pairs}),
            q::equivalence_questions(chunk.len()),
            cancel,
        )?;
        usage.add(&response);
        for (i, c) in chunk.iter().enumerate() {
            let (relation, confidence) =
                chosen(answer(&response.answers, &format!("p{i}::relation"))?)?;
            if relation == "same" && confidence >= q::EQUIVALENT {
                out.push((c.first.clone(), c.second.clone()));
            }
        }
    }
    Ok((out, usage))
}

/// Reduce: align organizations that share a leading name word. Returns
/// (a, b, normalized score, propose) for every judged pair.
pub fn align_organizations(
    candidates: &[MergeCandidate],
    decisions: &mut impl Decisions,
    cancel: &AtomicBool,
) -> Result<(Vec<Alignment>, StageUsage)> {
    let mut usage = StageUsage::default();
    let mut out = Vec::new();
    for chunk in candidates.chunks(20) {
        let pairs: Map<String, Value> = chunk
            .iter()
            .enumerate()
            .map(|(i, c)| {
                (
                    format!("p{i}"),
                    json!({"entity_a":{"name":c.a_label},"entity_b":{"name":c.b_label}}),
                )
            })
            .collect();
        let response = decisions.evaluate(
            json!({"pairs":pairs}),
            q::alignment_questions(chunk.len()),
            cancel,
        )?;
        usage.add(&response);
        for (i, c) in chunk.iter().enumerate() {
            let score = answer(&response.answers, &format!("p{i}::link_state"))?["score"]
                .as_f64()
                .ok_or_else(invalid)?;
            // Score rounding: 0 different, 1 curator, 2 same. Never an automatic merge.
            out.push(Alignment {
                a: c.a.clone(),
                b: c.b.clone(),
                score: score / 2.,
                propose: score >= q::ALIGN_PROPOSE,
            });
        }
    }
    Ok((out, usage))
}

/// On demand: which lazy documents answer a question. Several documents share one
/// request, each with its own existence Noul. Returns the matching document IDs.
pub fn search_documents(
    query: &str,
    documents: &[(String, String, String)],
    decisions: &mut impl Decisions,
    cancel: &AtomicBool,
) -> Result<(Vec<String>, StageUsage)> {
    let mut usage = StageUsage::default();
    let mut found = Vec::new();
    let mut start = 0;
    while start < documents.len() {
        let mut bytes = 0;
        let mut end = start;
        while end < documents.len() && end - start < 40 {
            let (_, title, excerpt) = &documents[end];
            let size = title.len() + excerpt.len().min(2_000);
            if bytes + size > SEARCH_STATE_BYTES && end > start {
                break;
            }
            bytes += size;
            end += 1;
        }
        let batch = &documents[start..end];
        let docs: Map<String, Value> = batch
            .iter()
            .enumerate()
            .map(|(i, (_, title, excerpt))| {
                (
                    format!("d{i}"),
                    json!({"title":title,"excerpt":prefix(excerpt, 2_000)}),
                )
            })
            .collect();
        let response = decisions.evaluate(
            json!({"query":query,"documents":docs}),
            q::search_questions(batch.len()),
            cancel,
        )?;
        usage.add(&response);
        for (i, (id, _, _)) in batch.iter().enumerate() {
            if noul(answer(&response.answers, &format!("doc_{i}"))?)? >= q::SEARCH_FOUND {
                found.push(id.clone());
            }
        }
        start = end;
    }
    Ok((found, usage))
}

#[cfg(test)]
#[path = "graph_pipeline_tests.rs"]
mod tests;
