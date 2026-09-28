//! TypeSafe validation of every assumption the reader made about a fact: that
//! the value means what its label says, whose it is, which period it covers and
//! which profile slot it fills. Code combines the answers and decides the band.
use crate::graph_pipeline::{StageUsage, invalid};
use crate::typesafe::{Decisions, Result};
use crate::typesafe_questions as q;
use me_core::{IdentityAnchors, Period, Slot, Target, ValueKind};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;

/// One fact the reader returned, as verification sees it.
pub struct FactInput {
    pub label: String,
    pub value: String,
    pub quote: String,
    pub line: String,
    /// The value is printed as an amount, or is a whole amount the reader tagged
    /// for a money slot (`read_assembly::looks_like_money`); the period question
    /// applies. Digit-only identifiers are not amounts.
    pub money: bool,
    /// Slot the reader tagged and the value type-checked for.
    pub slot: Option<&'static Slot>,
    /// A category slot whose printed value code could not map.
    pub category_unmapped: bool,
}

/// Whose detail a fact is.
#[derive(Clone, Debug, PartialEq)]
pub enum Owner {
    /// An identity anchor (self or a household member), by entity id.
    Anchor(String),
    /// Another person named in the document, by printed name.
    Party(String),
    Organization,
    Unclear,
}
/// The period an amount covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeriodKind {
    Document,
    Cumulative,
    Other,
    NotPeriodic,
}
/// Accept silently, ask a Quick check, or reject the assumption.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Band {
    Accept,
    Check,
    Reject,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Verdict {
    pub owner: Owner,
    pub period: Option<PeriodKind>,
    /// Highest failure-mode probability (true means wrong).
    pub failure: f64,
    /// Weakest head.
    pub confidence: f64,
    pub band: Band,
    /// The fact fills its slot in the profile.
    pub mapped: bool,
    pub refund: Option<bool>,
    pub payment_period: Option<Period>,
    pub category: Option<String>,
}
pub struct Verification {
    /// One verdict per input fact, in input order.
    pub verdicts: Vec<Verdict>,
    /// The document states that it corrects, replaces or cancels an earlier one.
    pub correction: bool,
    pub usage: StageUsage,
}

/// Band of one fact: a fired failure head or a confidence below `SLOT_FLOOR`
/// rejects; below the vault's check threshold asks; otherwise accepts.
pub fn band(confidence: f64, failure: f64, threshold: f64) -> Band {
    if failure > q::VERIFY_FIRE || confidence < q::SLOT_FLOOR {
        Band::Reject
    } else if confidence < threshold {
        Band::Check
    } else {
        Band::Accept
    }
}

fn chosen(a: &Value, key: &str) -> Result<Option<(String, f64)>> {
    let Some(v) = a.get(key) else {
        return Ok(None);
    };
    Ok(Some((
        v["choice"].as_str().ok_or_else(invalid)?.to_owned(),
        v["confidence"].as_f64().ok_or_else(invalid)?,
    )))
}
fn noul(a: &Value, key: &str) -> Result<Option<f64>> {
    match a.get(key) {
        None => Ok(None),
        Some(v) => Ok(Some(v["noul"].as_f64().ok_or_else(invalid)?)),
    }
}
/// A judgment that decides the stored value: its confidence always counts towards
/// the fact's, its key only at or above `SLOT_FLOOR`.
fn judged(answer: Option<(String, f64)>, confidence: &mut f64) -> Option<String> {
    let (key, c) = answer?;
    *confidence = confidence.min(c);
    (c >= q::SLOT_FLOOR).then_some(key)
}
/// The answers to exactly the asked questions. A missing answer, or a Choice key
/// outside its criteria, makes the whole response invalid.
fn answered(questions: &Value, answers: &Value) -> Result<Value> {
    let mut out = Map::new();
    for (key, question) in questions.as_object().ok_or_else(invalid)? {
        let answer = answers.get(key).ok_or_else(invalid)?;
        if question["type"] == "choice" {
            let option = answer["choice"].as_str().ok_or_else(invalid)?;
            if question["criteria"].get(option).is_none() {
                return Err(invalid());
            }
        }
        out.insert(key.clone(), answer.clone());
    }
    Ok(Value::Object(out))
}

/// The start of a document as verification sees it: its first `LETTERHEAD_LINES`
/// lines, at most `LETTERHEAD_BYTES` bytes, cut at a character boundary.
pub fn letterhead(text: &str) -> &str {
    let lines = text
        .match_indices('\n')
        .nth(q::LETTERHEAD_LINES - 1)
        .map_or(text.len(), |(i, _)| i);
    let mut end = lines.min(q::LETTERHEAD_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn state(title: &str, doc_label: &str, excerpt: &str, batch: &[(usize, &FactInput)]) -> Value {
    let mut facts = Map::new();
    let mut slots = Map::new();
    for (i, f) in batch {
        facts.insert(
            format!("f{i}"),
            json!({"label":f.label,"value":f.value,"quote":f.quote,"line":f.line}),
        );
        if let Some(s) = f.slot {
            slots.insert(
                s.key.to_owned(),
                json!({"label":s.label,"description":s.description}),
            );
        }
    }
    json!({
        "document":{"file_name":title,"type":doc_label,"excerpt":excerpt},
        "facts":facts,
        "slots":slots
    })
}

/// Verifies every reader fact in batches of `FACT_BATCH`, then decides each slot
/// that more than one mapped fact claims with one Choice over those facts.
/// `excerpt` is the document's opening text; only its `letterhead` is sent, as
/// `document.excerpt`, with every request.
#[allow(clippy::too_many_arguments)]
pub fn verify_facts(
    title: &str,
    doc_label: &str,
    excerpt: &str,
    facts: &[FactInput],
    anchors: &IdentityAnchors,
    named: &[String],
    subject: Option<&str>,
    threshold: f64,
    decisions: &mut impl Decisions,
    cancel: &AtomicBool,
) -> Result<Verification> {
    let excerpt = letterhead(excerpt);
    let owners = q::owner_options(anchors, named);
    let mut usage = StageUsage::default();
    let mut verdicts = Vec::with_capacity(facts.len());
    let mut correction = false;
    let indexed: Vec<(usize, &FactInput)> = facts.iter().enumerate().collect();
    for (n, batch) in indexed.chunks(q::FACT_BATCH).enumerate() {
        let questions = q::fact_questions(batch, &owners, n == 0);
        let response = decisions.evaluate(
            state(title, doc_label, excerpt, batch),
            questions.clone(),
            cancel,
        )?;
        usage.add(&response);
        // Only asked questions count; each has an answer with a recognised key.
        let a = &answered(&questions, &response.answers)?;
        if n == 0 {
            correction = noul(a, "document::correction")?.is_some_and(|p| p >= q::PRESENT_FOUND);
        }
        for (i, fact) in batch {
            let key = |head: &str| format!("f{i}::{head}");
            // Both failure heads and the owner are always asked, so always answered.
            let mut failure: f64 = 0.;
            for head in ["hallucinated", "off_target"] {
                failure = failure.max(noul(a, &key(head))?.ok_or_else(invalid)?);
            }
            let (owner_key, owner_confidence) = chosen(a, &key("owner"))?.ok_or_else(invalid)?;
            let owner =
                if let Some((_, entity)) = owners.anchors.iter().find(|(k, _)| *k == owner_key) {
                    Owner::Anchor(entity.clone())
                } else if let Some((_, name)) = owners.named.iter().find(|(k, _)| *k == owner_key) {
                    Owner::Party(name.clone())
                } else {
                    match owner_key.as_str() {
                        "organization" => Owner::Organization,
                        "unclear" => Owner::Unclear,
                        _ => return Err(invalid()),
                    }
                };
            let period = chosen(a, &key("period"))?;
            let period_kind = match period.as_ref().map(|(k, _)| k.as_str()) {
                None => None,
                Some("document") => Some(PeriodKind::Document),
                Some("cumulative") => Some(PeriodKind::Cumulative),
                Some("other") => Some(PeriodKind::Other),
                Some("not_periodic") => Some(PeriodKind::NotPeriodic),
                Some(_) => return Err(invalid()),
            };
            let mapping = noul(a, &key("mapping"))?;
            // The weakest head decides, including each asked judgment that decides
            // the stored value: direction, payment period and category.
            let mut confidence = (1. - failure).min(owner_confidence);
            for c in period.iter().map(|(_, c)| *c).chain(mapping) {
                confidence = confidence.min(c);
            }
            let refund = match judged(chosen(a, &key("direction"))?, &mut confidence).as_deref() {
                Some("refund") => Some(true),
                Some("payment") => Some(false),
                Some("unclear") | None => None,
                Some(_) => return Err(invalid()),
            };
            let payment_period =
                match judged(chosen(a, &key("payment_period"))?, &mut confidence).as_deref() {
                    Some("month") => Some(Period::Month),
                    Some("quarter") => Some(Period::Quarter),
                    Some("half_year") => Some(Period::HalfYear),
                    Some("year") => Some(Period::Year),
                    Some("once" | "none") | None => None,
                    Some(_) => return Err(invalid()),
                };
            let category =
                judged(chosen(a, &key("category"))?, &mut confidence).filter(|k| k != "other");
            let fact_band = band(confidence, failure, threshold);
            let mapped = fact.slot.is_some_and(|slot| {
                let owner_ok = match slot.target {
                    Target::Subject => {
                        matches!(&owner, Owner::Anchor(e) if Some(e.as_str()) == subject)
                    }
                    Target::Role(_) => true,
                };
                let period_ok = match slot.value {
                    ValueKind::Money(Some(_)) => period_kind == Some(PeriodKind::Document),
                    ValueKind::Balance => {
                        matches!(
                            period_kind,
                            Some(PeriodKind::Document | PeriodKind::NotPeriodic)
                        ) && refund.is_some()
                    }
                    ValueKind::Money(None) => period_kind != Some(PeriodKind::Cumulative),
                    ValueKind::Category(_) => !fact.category_unmapped || category.is_some(),
                    _ => true,
                };
                owner_ok
                    && period_ok
                    && mapping.is_some_and(|m| m >= q::SLOT_FLOOR)
                    && fact_band != Band::Reject
            });
            verdicts.push(Verdict {
                owner,
                period: period_kind,
                failure,
                confidence,
                band: fact_band,
                mapped,
                refund,
                payment_period,
                category,
            });
        }
    }
    // Competing mappings: one Choice per contested slot over the reader's facts.
    let mut contested: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, (fact, verdict)) in facts.iter().zip(&verdicts).enumerate() {
        if let Some(slot) = fact.slot.filter(|_| verdict.mapped) {
            contested.entry(slot.key).or_default().push(i);
        }
    }
    for (slot, ids) in contested.into_iter().filter(|(_, ids)| ids.len() > 1) {
        // Bound: a Choice offers at most `MAX_SLOT_OPTIONS` (60) facts, the first
        // ones read, well inside the Choice limit; later claimants stay unmapped.
        let offered = &ids[..ids.len().min(q::MAX_SLOT_OPTIONS)];
        let batch: Vec<(usize, &FactInput)> = offered.iter().map(|&i| (i, &facts[i])).collect();
        let questions = q::competing_questions(slot, offered);
        let response = decisions.evaluate(
            state(title, doc_label, excerpt, &batch),
            questions.clone(),
            cancel,
        )?;
        usage.add(&response);
        let a = answered(&questions, &response.answers)?;
        let (choice, c) = chosen(&a, &format!("slot_{slot}"))?.ok_or_else(invalid)?;
        let winner = match choice.as_str() {
            "none" => None,
            key => Some(
                offered
                    .iter()
                    .copied()
                    .find(|i| format!("f{i}") == key)
                    .ok_or_else(invalid)?,
            ),
        }
        .filter(|_| c >= q::SLOT_FLOOR);
        for &i in &ids {
            verdicts[i].mapped = Some(i) == winner;
        }
        // The Choice decides the stored value, so it bounds the winner's confidence:
        // a doubtful choice turns an accepted fact into a check.
        if let Some(i) = winner {
            let v = &mut verdicts[i];
            v.confidence = v.confidence.min(c);
            v.band = band(v.confidence, v.failure, threshold);
        }
    }
    Ok(Verification {
        verdicts,
        correction,
        usage,
    })
}

#[cfg(test)]
#[path = "fact_verification_tests.rs"]
mod tests;
