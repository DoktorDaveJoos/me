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
    /// The value parses as an amount; the period question applies.
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
/// A Choice key, only when its confidence reaches `SLOT_FLOOR`.
fn confident(answer: Option<(String, f64)>) -> Option<String> {
    answer.and_then(|(k, c)| (c >= q::SLOT_FLOOR).then_some(k))
}

fn state(title: &str, doc_label: &str, batch: &[(usize, &FactInput)]) -> Value {
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
    json!({"document":{"file_name":title,"type":doc_label},"facts":facts,"slots":slots})
}

/// Verifies every reader fact in batches of `FACT_BATCH`, then decides each slot
/// that more than one mapped fact claims with one Choice over those facts.
#[allow(clippy::too_many_arguments)]
pub fn verify_facts(
    title: &str,
    doc_label: &str,
    facts: &[FactInput],
    anchors: &IdentityAnchors,
    named: &[String],
    subject: Option<&str>,
    threshold: f64,
    decisions: &mut impl Decisions,
    cancel: &AtomicBool,
) -> Result<Verification> {
    let owners = q::owner_options(anchors, named);
    let mut usage = StageUsage::default();
    let mut verdicts = Vec::with_capacity(facts.len());
    let mut correction = false;
    let indexed: Vec<(usize, &FactInput)> = facts.iter().enumerate().collect();
    for (n, batch) in indexed.chunks(q::FACT_BATCH).enumerate() {
        let response = decisions.evaluate(
            state(title, doc_label, batch),
            q::fact_questions(batch, &owners, n == 0),
            cancel,
        )?;
        usage.add(&response);
        let a = &response.answers;
        if n == 0 {
            correction = noul(a, "document::correction")?.is_some_and(|p| p >= q::PRESENT_FOUND);
        }
        for (i, fact) in batch {
            let f = format!("f{i}");
            // Both failure heads are always asked; a missing one fails closed.
            let mut failure: f64 = 0.;
            for head in ["hallucinated", "off_target"] {
                let p = noul(a, &format!("{f}::{head}"))?.ok_or_else(invalid)?;
                failure = failure.max(p);
            }
            let (owner_key, owner_confidence) =
                chosen(a, &format!("{f}::owner"))?.ok_or_else(invalid)?;
            let owner =
                if let Some((_, entity)) = owners.anchors.iter().find(|(k, _)| *k == owner_key) {
                    Owner::Anchor(entity.clone())
                } else if let Some((_, name)) = owners.named.iter().find(|(k, _)| *k == owner_key) {
                    Owner::Party(name.clone())
                } else if owner_key == "organization" {
                    Owner::Organization
                } else {
                    Owner::Unclear
                };
            let period = chosen(a, &format!("{f}::period"))?;
            let period_kind = period.as_ref().map(|(k, _)| match k.as_str() {
                "document" => PeriodKind::Document,
                "cumulative" => PeriodKind::Cumulative,
                "other" => PeriodKind::Other,
                _ => PeriodKind::NotPeriodic,
            });
            let mapping = noul(a, &format!("{f}::mapping"))?;
            // The weakest head decides.
            let mut confidence = (1. - failure).min(owner_confidence);
            if let Some((_, c)) = &period {
                confidence = confidence.min(*c);
            }
            if let Some(m) = mapping {
                confidence = confidence.min(m);
            }
            let fact_band = band(confidence, failure, threshold);
            let refund = match confident(chosen(a, &format!("{f}::direction"))?).as_deref() {
                Some("refund") => Some(true),
                Some("payment") => Some(false),
                _ => None,
            };
            let payment_period =
                match confident(chosen(a, &format!("{f}::payment_period"))?).as_deref() {
                    Some("month") => Some(Period::Month),
                    Some("quarter") => Some(Period::Quarter),
                    Some("half_year") => Some(Period::HalfYear),
                    Some("year") => Some(Period::Year),
                    _ => None,
                };
            let category =
                confident(chosen(a, &format!("{f}::category"))?).filter(|k| k != "other");
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
        let batch: Vec<(usize, &FactInput)> = ids.iter().map(|i| (*i, &facts[*i])).collect();
        let response = decisions.evaluate(
            state(title, doc_label, &batch),
            q::competing_questions(slot, &ids),
            cancel,
        )?;
        usage.add(&response);
        let (winner, c) =
            chosen(&response.answers, &format!("slot_{slot}"))?.ok_or_else(invalid)?;
        for i in ids {
            if c < q::SLOT_FLOOR || winner != format!("f{i}") {
                verdicts[i].mapped = false;
            }
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
