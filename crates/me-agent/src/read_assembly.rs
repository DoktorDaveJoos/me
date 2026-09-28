//! The pure parts of deep-first reading: locating and typing reader facts,
//! turning TypeSafe verdicts into stored document facts and profile values, the
//! refund sign, machine-readable-zone fields and the pay-month fallback. No vault
//! session, provider or UI is involved; the desktop orchestrates these steps.
use crate::fact_verification::{Band, FactInput, Owner, PeriodKind, Verdict};
use me_core::{
    Candidate, CandidateKind, CandidateValue, ConfidenceSource, DocType, DocumentRead,
    ExtractedFact, FactState, ImportErrorKind, Located, ReadFact, RejectedFact, Slot, SlotContent,
    SlotValue, SourceSegment, TypingContext, Uncovered, ValueKind, doc_types::MrzField, locate,
    type_value,
};
use std::collections::BTreeMap;

/// Other people named in a document offered as owners, at most.
pub const NAMED_OWNERS: usize = 24;

/// People the local scanner found named in the document, deduplicated in document
/// order, at most [`NAMED_OWNERS`].
pub fn named_owners(candidates: &[Candidate]) -> Vec<String> {
    let mut named: Vec<String> = Vec::new();
    for c in candidates
        .iter()
        .filter(|c| c.kind == CandidateKind::PersonName)
    {
        let name = c.text.trim();
        if named.len() < NAMED_OWNERS && !named.iter().any(|n| n == name) {
            named.push(name.to_owned());
        }
    }
    named
}

/// Source spans (segment, quote start, quote end) that reader facts quote.
pub fn covered_spans(
    segments: &[SourceSegment<'_>],
    facts: &[ExtractedFact],
) -> Vec<(String, usize, usize)> {
    facts
        .iter()
        .filter_map(|f| locate(segments, &f.segment_id, &f.quote, &f.value))
        .map(|l| (l.segment_id, l.quote_start, l.quote_end))
        .collect()
}

/// Facts whose only open question is their owner stay (verification asks it);
/// every other grounding rejection is counted by code, never stored.
pub fn keep_grounded(
    facts: &mut Vec<ExtractedFact>,
    rejected: &mut BTreeMap<String, usize>,
    incoming: Vec<RejectedFact>,
) {
    for r in incoming {
        if r.code == "subject_unknown" {
            me_core::merge_extracted_fact(facts, r.fact);
        } else {
            *rejected.entry(r.code.to_owned()).or_default() += 1;
        }
    }
}

/// A reader fact at its exact source span, with its value typed for the slot it
/// was tagged with when the printed value can hold that slot's kind.
pub struct Typed<'f> {
    pub fact: &'f ExtractedFact,
    pub at: Located,
    pub candidate: Option<Candidate>,
}

/// The printed label of a reader property.
pub fn fact_label(property: &str) -> String {
    me_core::document_field_label(property)
        .map(str::to_owned)
        .or_else(|| {
            me_core::STANDARD_FIELDS
                .iter()
                .find(|s| s.key == property)
                .map(|s| s.label.to_owned())
        })
        .unwrap_or_else(|| property.to_owned())
}

/// Whether a fact's value is an amount, so verification asks which period it
/// covers. A value printed as money says so itself: a currency marker, exactly
/// two decimals, or a joined EUR/Ct cell pair under an EUR/Ct header. A whole
/// number counts only when the reader tagged a money slot, where it types as a
/// whole amount; untagged, a digit-only value is an identifier (a tax ID, a
/// personnel number) and is never asked the period question.
pub fn looks_like_money(typing: &TypingContext, value: &str, slot: Option<&Slot>) -> bool {
    if slot.is_some_and(|s| matches!(s.value, ValueKind::Money(_) | ValueKind::Balance)) {
        return me_core::amount_of(typing, value).is_some();
    }
    if me_core::parse_bare_amount(value, typing.euro_cent, false).is_some() {
        return true;
    }
    let printed = [SourceSegment {
        id: "v",
        text: value,
    }];
    me_core::find_candidates(&printed, &[], typing.year)
        .iter()
        .any(|c| c.kind == CandidateKind::Money && c.text.trim() == value.trim())
}

/// Locates and types every reader fact. Facts whose quote or value is not
/// verbatim in their segment are left out. A slot tag is kept only when the value
/// type-checks for it, or when it names a category code could not map.
pub fn type_facts<'f>(
    facts: &'f [ExtractedFact],
    segments: &[SourceSegment<'_>],
    typing: &TypingContext,
    kind: Option<&'static DocType>,
) -> (Vec<Typed<'f>>, Vec<FactInput>) {
    let mut typed = Vec::new();
    let mut inputs = Vec::new();
    for fact in facts {
        let Some(at) = locate(segments, &fact.segment_id, &fact.quote, &fact.value) else {
            continue;
        };
        let label = fact_label(&fact.property);
        // Machine-readable-zone slots are filled by position, never from a tag.
        let slot = kind
            .and_then(|k| k.slot(&fact.slot))
            .filter(|s| s.mrz.is_none());
        let money = looks_like_money(typing, &fact.value, slot);
        let candidate = slot.and_then(|s| type_value(typing, &at, &label, &fact.value, s.value));
        let category_unmapped =
            matches!(slot.map(|s| s.value), Some(ValueKind::Category(_))) && candidate.is_none();
        let slot = slot.filter(|_| candidate.is_some() || category_unmapped);
        inputs.push(FactInput {
            label,
            value: fact.value.clone(),
            quote: fact.quote.clone(),
            line: at.line.clone(),
            money,
            slot,
            category_unmapped,
        });
        typed.push(Typed {
            fact,
            at,
            candidate,
        });
    }
    (typed, inputs)
}

/// Accepted assumptions are verified; a check or a rejected assumption keeps the
/// grounded fact on its document as uncertain.
pub fn fact_state(band: Band) -> FactState {
    match band {
        Band::Accept => FactState::Verified,
        Band::Check | Band::Reject => FactState::Uncertain,
    }
}

fn period_key(period: PeriodKind) -> &'static str {
    match period {
        PeriodKind::Document => "document",
        PeriodKind::Cumulative => "cumulative",
        PeriodKind::Other => "other",
        PeriodKind::NotPeriodic => "none",
    }
}

/// The stored document fact of one verified reader fact. It is linked to the
/// profile only when its mapping was verified.
pub fn read_fact(t: &Typed<'_>, input: &FactInput, v: &Verdict, self_entity: &str) -> ReadFact {
    let (owner, owner_entity, owner_name) = match &v.owner {
        Owner::Anchor(e) if e == self_entity => ("self", Some(e.clone()), None),
        Owner::Anchor(e) => ("household", Some(e.clone()), None),
        Owner::Party(name) => ("party", None, Some(name.clone())),
        Owner::Organization => ("organization", None, None),
        Owner::Unclear => ("unclear", None, None),
    };
    ReadFact {
        label: input.label.clone(),
        value: input.value.clone(),
        segment_id: t.at.segment_id.clone(),
        start: t.at.start,
        end: t.at.end,
        context: t.fact.context_quote.clone(),
        owner: owner.into(),
        owner_entity,
        owner_name,
        period: v.period.map(|p| period_key(p).to_owned()),
        slot: input.slot.filter(|_| v.mapped).map(|s| s.key.to_owned()),
        state: fact_state(v.band),
        confidence: Some(v.confidence),
    }
}

/// A grounded fact whose assumptions could not be verified: kept on its
/// document, owner unknown, never in the profile.
pub fn unverified_fact(t: &Typed<'_>, input: &FactInput) -> ReadFact {
    ReadFact {
        label: input.label.clone(),
        value: input.value.clone(),
        segment_id: t.at.segment_id.clone(),
        start: t.at.start,
        end: t.at.end,
        context: t.fact.context_quote.clone(),
        owner: "unknown".into(),
        owner_entity: None,
        owner_name: None,
        period: None,
        slot: None,
        state: FactState::Unverified,
        confidence: None,
    }
}

/// What a read stores when its verification failed: every grounded fact as
/// unverified (owner unknown, never linked to the profile), the values nobody
/// interpreted and the rejection counts, but no profile values. The vault stores
/// it without touching the source's existing profile values. A cancelled
/// verification stores nothing.
pub fn unverified_read(
    run_id: &str,
    typed: &[Typed<'_>],
    inputs: &[FactInput],
    uninterpreted: Vec<Uncovered>,
    rejected: BTreeMap<String, usize>,
    failure: ImportErrorKind,
) -> Option<DocumentRead> {
    (failure != ImportErrorKind::Cancelled).then(|| DocumentRead {
        run_id: run_id.to_owned(),
        graph: None,
        facts: typed
            .iter()
            .zip(inputs)
            .map(|(t, i)| unverified_fact(t, i))
            .collect(),
        uninterpreted,
        rejected,
    })
}

/// Code applies the direction TypeSafe judged: a refund is stored negative and
/// a payment positive, whatever sign was printed. Zero stays unsigned.
pub fn apply_direction(c: &mut Candidate, refund: Option<bool>) {
    let (Some(refund), CandidateValue::Money { amount, .. }) = (refund, &mut c.value) else {
        return;
    };
    let magnitude = amount.trim_start_matches(['-', '+']).to_owned();
    let zero = magnitude.bytes().all(|b| b == b'0' || b == b'.');
    *amount = if refund && !zero {
        format!("-{magnitude}")
    } else {
        magnitude
    };
}

/// Profile values of an eager type: machine-readable-zone fields by position,
/// then every verified mapping, then the pay-month fallback.
pub fn profile_values(
    kind: &DocType,
    candidates: &[Candidate],
    segments: &[SourceSegment<'_>],
    typed: &[Typed<'_>],
    inputs: &[FactInput],
    verdicts: &[Verdict],
) -> Vec<SlotValue> {
    let mut values: Vec<SlotValue> = Vec::new();
    if let Some(mrz) = candidates
        .iter()
        .find(|c| matches!(&c.value, CandidateValue::Mrz(m) if m.check_digits_valid))
    {
        for slot in kind.slots {
            if let Some(field) = slot.mrz
                && let Some(c) = mrz_value(mrz, field)
            {
                values.push(SlotValue {
                    slot: slot.key.into(),
                    content: SlotContent::Candidate(Box::new(c)),
                    period: None,
                    confidence: 1.,
                    source: ConfidenceSource::Checksum,
                    value_checked: true,
                    check: false,
                });
            }
        }
    }
    for ((t, input), v) in typed.iter().zip(inputs).zip(verdicts) {
        let (Some(slot), true) = (input.slot, v.mapped) else {
            continue;
        };
        if values.iter().any(|x| x.slot == slot.key) {
            continue;
        }
        let content = match (&t.candidate, &v.category) {
            (None, Some(key)) if input.category_unmapped => {
                SlotContent::Category { key: key.clone() }
            }
            (Some(c), _) => {
                let mut c = c.clone();
                if slot.value == ValueKind::Balance {
                    apply_direction(&mut c, v.refund);
                }
                SlotContent::Candidate(Box::new(c))
            }
            _ => continue,
        };
        values.push(SlotValue {
            slot: slot.key.into(),
            content,
            period: v.payment_period,
            confidence: v.confidence,
            source: ConfidenceSource::Typesafe,
            value_checked: t.candidate.as_ref().is_some_and(|c| c.checksum),
            check: v.band == Band::Check,
        });
    }
    if let Some(slot) = kind.slots.iter().find(|s| s.value == ValueKind::Period)
        && !values.iter().any(|x| x.slot == slot.key)
        && let Some((period, confidence)) =
            shared_period(candidates, segments, typed, inputs, verdicts)
    {
        values.push(SlotValue {
            slot: slot.key.into(),
            content: SlotContent::Candidate(Box::new(period)),
            period: None,
            confidence,
            source: ConfidenceSource::Typesafe,
            value_checked: false,
            check: false,
        });
    }
    values
}

/// Where a fact's context quote sits: in the fact's own segment, else in the
/// first segment that prints it.
fn locate_context(segments: &[SourceSegment<'_>], fact: &ExtractedFact) -> Option<Located> {
    let quote = fact.context_quote.as_str();
    if quote.trim().is_empty() {
        return None;
    }
    locate(segments, &fact.segment_id, quote, quote).or_else(|| {
        segments
            .iter()
            .find_map(|s| locate(segments, s.id, quote, quote))
    })
}

/// Pay-month fallback: the one period that the context quotes of all mapped
/// amounts for this document's own period print, at its real source span, with
/// the weakest confidence among those amounts. Only a period of the amounts' own
/// granularity counts: a monthly amount's context gives a whole calendar month
/// (the pay month), a yearly amount's a whole calendar year (the tax year), so a
/// month token never fills a tax year. `None` when they print no such period or
/// more than one.
fn shared_period(
    candidates: &[Candidate],
    segments: &[SourceSegment<'_>],
    typed: &[Typed<'_>],
    inputs: &[FactInput],
    verdicts: &[Verdict],
) -> Option<(Candidate, f64)> {
    let mut found: Vec<&Candidate> = Vec::new();
    let mut confidence: f64 = 1.;
    for ((t, input), v) in typed.iter().zip(inputs).zip(verdicts) {
        if !(v.mapped && input.money && v.period == Some(PeriodKind::Document)) {
            continue;
        }
        let Some(ValueKind::Money(Some(granularity))) = input.slot.map(|s| s.value) else {
            continue;
        };
        let Some(at) = locate_context(segments, t.fact) else {
            continue;
        };
        let periods: Vec<&Candidate> = candidates
            .iter()
            .filter(|c| {
                c.kind == CandidateKind::Period
                    && c.segment_id == at.segment_id
                    && at.quote_start <= c.start
                    && c.end <= at.quote_end
                    && matches!(&c.value, CandidateValue::Period { start, end }
                        if me_core::whole_period(granularity, start, end))
            })
            .collect();
        if !periods.is_empty() {
            confidence = confidence.min(v.confidence);
            found.extend(periods);
        }
    }
    let first = *found.first()?;
    found
        .iter()
        .all(|c| c.value == first.value)
        .then(|| (first.clone(), confidence))
}

/// The value of one machine-readable-zone field, at the zone's span. Position
/// defines the meaning; the check digits validate the value.
pub fn mrz_value(c: &Candidate, field: MrzField) -> Option<Candidate> {
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

/// Current UTC year, for the century of two-digit MRZ years.
pub fn time_year() -> i32 {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400) as i64;
    // Civil-from-days (Howard Hinnant), year only.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    (yoe + era * 400 + i64::from(mp >= 10)) as i32
}

#[cfg(test)]
#[path = "read_assembly_tests.rs"]
mod tests;
