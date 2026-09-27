//! Import stages that ask TypeSafe typed questions: Classify (one request per
//! document), Extract (select instead of generate: one request per eager document),
//! verification of values proposed by another model, and the reduce-side checks
//! (value equivalence, organization alignment, on-demand search). Code owns control
//! flow, dates and arithmetic; TypeSafe answers narrow semantic questions.
use crate::typesafe::{Decisions, Result};
use crate::typesafe_questions as q;
use me_core::{
    Candidate, CandidateKind, CandidateValue, ConfidenceSource, DocType, DocumentGraph,
    IdentityAnchors, ImportErrorKind as Kind, ImportFailure, ImportProvider, MergeCandidate,
    Period, SlotContent, SlotValue, SourceSegment, ValueConflict, ValueKind,
    doc_types::{self, MrzField, REGISTRY_VERSION},
    normalize_name,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::sync::atomic::AtomicBool;

pub const CLASSIFY_STAGE: &str = "classify";
pub const EXTRACT_STAGE: &str = "extract";
/// Bytes of the first and the last page sent for classification.
const CLASSIFY_PAGE_BYTES: usize = 6_000;
/// Bytes of document excerpts per on-demand search request.
const SEARCH_STATE_BYTES: usize = 20_000;

fn invalid() -> ImportFailure {
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
    fn add(&mut self, r: &crate::typesafe::DecisionResponse) {
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
pub fn extract_version(doc_type: &str, subject: Option<&str>) -> String {
    format!(
        "extract-v1|{REGISTRY_VERSION}|{}|{doc_type}|{}",
        q::MODEL,
        subject.unwrap_or("-")
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
    /// The eager type to extract, if classification was confident enough.
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

/// Lines of the document with their segment and byte range.
struct Line<'a> {
    segment: &'a str,
    start: usize,
    end: usize,
    text: &'a str,
}
fn lines<'a>(segments: &'a [SourceSegment<'a>]) -> Vec<Line<'a>> {
    let mut out = Vec::new();
    for s in segments {
        let mut start = 0;
        for part in s.text.split_inclusive('\n') {
            let text = part.trim_end_matches(['\n', '\r']);
            out.push(Line {
                segment: s.id,
                start,
                end: start + part.len(),
                text,
            });
            start += part.len();
        }
    }
    out
}
/// The first lines (letterhead) plus each candidate line with one line of context.
fn extract_state(
    title: &str,
    label: &str,
    segments: &[SourceSegment<'_>],
    used: &[&Candidate],
) -> Value {
    let all = lines(segments);
    let mut keep = std::collections::BTreeSet::new();
    for i in 0..all.len().min(12) {
        keep.insert(i);
    }
    for c in used {
        if let Some(i) = all.iter().position(|l| {
            l.segment == c.segment_id && l.start <= c.start && c.start < l.end.max(l.start + 1)
        }) {
            keep.extend(i.saturating_sub(1)..=(i + 1).min(all.len().saturating_sub(1)));
        }
    }
    let mut bytes = 0;
    let mut out = Vec::new();
    for i in keep {
        let line = format!("L{i:03}| {}", all[i].text.trim());
        bytes += line.len();
        if bytes > q::MAX_EXTRACT_STATE {
            break;
        }
        out.push(line);
    }
    json!({"document":{"file_name":title,"type":label,"lines":out}})
}

fn mrz_value(c: &Candidate, field: MrzField) -> Option<Candidate> {
    let CandidateValue::Mrz(m) = &c.value else {
        return None;
    };
    if !m.check_digits_valid {
        return None;
    }
    let value = match field {
        MrzField::DocumentNumber => CandidateValue::Identifier(m.document_number.clone()),
        MrzField::BirthDate => CandidateValue::Date(m.birth_date.clone()?),
        MrzField::ExpiryDate => CandidateValue::Date(m.expiry_date.clone()?),
        MrzField::Nationality => CandidateValue::Identifier(m.nationality.clone()),
    };
    Some(Candidate { value, ..c.clone() })
}
fn period(key: &str) -> Option<Period> {
    Some(match key {
        "month" => Period::Month,
        "quarter" => Period::Quarter,
        "half_year" => Period::HalfYear,
        "year" => Period::Year,
        _ => return None,
    })
}

/// Result of Extract: the graph to resolve and required slots still missing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Extraction {
    pub graph: DocumentGraph,
    pub missing_required: Vec<String>,
    pub usage: StageUsage,
}

/// Select slot values from local candidates with one request per document.
/// `threshold` is the vault's current quick-check threshold.
pub fn extract(
    doc: Document<'_>,
    kind: &'static DocType,
    classification: &Classification,
    candidates: &[Candidate],
    threshold: f64,
    decisions: &mut impl Decisions,
    cancel: &AtomicBool,
) -> Result<Extraction> {
    let mut values = Vec::new();
    let mut usage = StageUsage::default();
    // Machine-readable zones: position defines meaning, check digits validate value.
    let mrz = candidates
        .iter()
        .find(|c| matches!(&c.value, CandidateValue::Mrz(m) if m.check_digits_valid));
    let mut local = Vec::new();
    for slot in kind.slots {
        if let (Some(field), Some(c)) = (slot.mrz, mrz)
            && let Some(value) = mrz_value(c, field)
        {
            local.push(slot.key);
            values.push(SlotValue {
                slot: slot.key.into(),
                content: SlotContent::Candidate(Box::new(value)),
                period: None,
                confidence: 1.,
                source: ConfidenceSource::Checksum,
                value_checked: true,
                check: false,
            });
        }
    }
    let (questions, offered) = q::extract_questions(kind, candidates, &local);
    if questions.as_object().is_some_and(|q| !q.is_empty()) {
        let used: Vec<&Candidate> = offered
            .iter()
            .flat_map(|(_, ids)| ids.iter())
            .filter_map(|id| candidates.iter().find(|c| &c.id == id))
            .collect();
        let response = decisions.evaluate(
            extract_state(doc.title, kind.label, doc.segments, &used),
            questions,
            cancel,
        )?;
        usage.add(&response);
        let a = &response.answers;
        for (slot_key, ids) in &offered {
            let slot = kind.slot(slot_key).ok_or_else(invalid)?;
            let (choice, confidence) = chosen(answer(a, &format!("slot_{slot_key}"))?)?;
            if confidence < q::SLOT_FLOOR || choice == "none" {
                continue;
            }
            if let ValueKind::Category(_) = slot.value {
                if choice != "other" {
                    values.push(SlotValue {
                        slot: slot_key.clone(),
                        content: SlotContent::Category { key: choice.into() },
                        period: None,
                        confidence,
                        source: ConfidenceSource::Typesafe,
                        value_checked: false,
                        check: confidence < threshold,
                    });
                }
                continue;
            }
            if !ids.iter().any(|id| id == choice) {
                return Err(invalid());
            }
            let present = noul(answer(a, &format!("present_{slot_key}"))?)?;
            if present < q::PRESENT_ABSENT {
                continue;
            }
            let candidate = candidates
                .iter()
                .find(|c| c.id == choice)
                .ok_or_else(invalid)?;
            let period = match a.get(format!("period_{slot_key}")) {
                Some(p) => {
                    let (key, c) = chosen(p)?;
                    if c >= q::SLOT_FLOOR {
                        period(key)
                    } else {
                        None
                    }
                }
                None => None,
            };
            values.push(SlotValue {
                slot: slot_key.clone(),
                content: SlotContent::Candidate(Box::new(candidate.clone())),
                period,
                confidence,
                source: ConfidenceSource::Typesafe,
                value_checked: candidate.checksum,
                check: confidence < threshold || present < q::PRESENT_FOUND,
            });
        }
    }
    let missing_required = kind
        .slots
        .iter()
        .filter(|s| s.required && !values.iter().any(|v| v.slot == s.key))
        .map(|s| s.key.to_owned())
        .collect();
    Ok(Extraction {
        graph: DocumentGraph {
            doc_type: kind.key.into(),
            subject: classification.subject.clone(),
            subject_confidence: classification.subject_confidence,
            values,
            models: usage.models.clone(),
        },
        missing_required,
        usage,
    })
}

/// Gap fill: another model proposes values for missing required slots. They are
/// offered to the same slot Choice as new candidates, then verified with the
/// SDE-cascade heads. Any head above the fire threshold rejects the value.
pub fn verify_gap_fill(
    doc: Document<'_>,
    kind: &'static DocType,
    proposed: &[Candidate],
    extraction: &mut Extraction,
    threshold: f64,
    decisions: &mut impl Decisions,
    cancel: &AtomicBool,
) -> Result<()> {
    if extraction.missing_required.is_empty() || proposed.is_empty() {
        return Ok(());
    }
    let missing: Vec<&str> = extraction
        .missing_required
        .iter()
        .map(String::as_str)
        .collect();
    let skip: Vec<&str> = kind
        .slots
        .iter()
        .map(|s| s.key)
        .filter(|k| !missing.contains(k))
        .collect();
    let (questions, offered) = q::extract_questions(kind, proposed, &skip);
    if questions.as_object().is_none_or(|q| q.is_empty()) {
        return Ok(());
    }
    let used: Vec<&Candidate> = proposed.iter().collect();
    let state = extract_state(doc.title, kind.label, doc.segments, &used);
    let response = decisions.evaluate(state.clone(), questions, cancel)?;
    extraction.usage.add(&response);
    let mut picked = Vec::new();
    for (slot_key, ids) in &offered {
        let (choice, confidence) = chosen(answer(&response.answers, &format!("slot_{slot_key}"))?)?;
        if choice == "none" || confidence < q::SLOT_FLOOR || !ids.iter().any(|i| i == choice) {
            continue;
        }
        let slot = kind.slot(slot_key).ok_or_else(invalid)?;
        let candidate = proposed
            .iter()
            .find(|c| c.id == choice)
            .ok_or_else(invalid)?;
        picked.push((slot, candidate, confidence));
    }
    if picked.is_empty() {
        return Ok(());
    }
    let fields: Vec<(String, Value, String)> = picked
        .iter()
        .map(|(slot, c, _)| {
            (
                slot.key.to_owned(),
                json!({"name":slot.label,"description":slot.description}),
                c.text.clone(),
            )
        })
        .collect();
    let verdict = decisions.evaluate(state, q::verify_questions(&fields), cancel)?;
    extraction.usage.add(&verdict);
    for (slot, candidate, confidence) in picked {
        let prefix = format!("{}::", slot.key);
        let worst = verdict
            .answers
            .as_object()
            .ok_or_else(invalid)?
            .iter()
            .filter(|(k, _)| k.starts_with(&prefix))
            .map(|(_, v)| noul(v))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .fold(0., f64::max);
        if worst > q::VERIFY_FIRE {
            continue;
        }
        extraction.graph.values.push(SlotValue {
            slot: slot.key.into(),
            content: SlotContent::Candidate(Box::new(candidate.clone())),
            period: None,
            confidence,
            source: ConfidenceSource::OpenaiGrounded,
            value_checked: candidate.checksum,
            check: confidence < threshold || worst > 1. - q::VERIFY_FIRE,
        });
        extraction.missing_required.retain(|m| m != slot.key);
    }
    extraction.graph.models = extraction.usage.models.clone();
    Ok(())
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
