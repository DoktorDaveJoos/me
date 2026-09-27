//! The pure parts of the read pipeline: typing reader facts, mapping verdicts to
//! stored facts and profile values, the refund sign and the pay-month fallback.
//! Synthetic data only.
use super::*;
use me_core::MrzData;

const SELF: &str = "00000000-0000-4000-8000-000000000001";
const PARTNER: &str = "00000000-0000-4000-8000-000000000002";
const PAYSLIP: &str = "Acme GmbH\nEntgeltabrechnung Januar 2026\nHerr Max Mustermann\nSteuerklasse drei\nGesamtbrutto 4.200,00 €\nNetto 2.800,00 €\nNachberechnung Dezember 2025\n";

fn seg(text: &str) -> Vec<SourceSegment<'_>> {
    vec![SourceSegment { id: "s1", text }]
}
fn fact(property: &str, value: &str, quote: &str, context: &str, slot: &str) -> ExtractedFact {
    ExtractedFact {
        property: property.into(),
        value: value.into(),
        segment_id: "s1".into(),
        quote: quote.into(),
        subject_quote: String::new(),
        context_quote: context.into(),
        slot: slot.into(),
    }
}
fn verdict(owner: Owner, band: Band, confidence: f64, mapped: bool) -> Verdict {
    Verdict {
        owner,
        period: Some(PeriodKind::Document),
        failure: 0.05,
        confidence,
        band,
        mapped,
        refund: None,
        payment_period: None,
        category: None,
    }
}
fn payslip_facts(net_context: &str) -> Vec<ExtractedFact> {
    vec![
        fact(
            "document.Gesamtbrutto",
            "4.200,00 €",
            "Gesamtbrutto 4.200,00 €",
            "Januar 2026",
            "gross",
        ),
        fact(
            "document.Netto",
            "2.800,00 €",
            "Netto 2.800,00 €",
            net_context,
            "net",
        ),
        fact(
            "document.Steuerklasse",
            "drei",
            "Steuerklasse drei",
            "",
            "tax_class",
        ),
        // Not verbatim in the segment: never typed or stored.
        fact("document.Bonus", "100,00 €", "Bonus 100,00 €", "", "none"),
        // Tagged, but the printed value cannot hold an amount.
        fact(
            "document.Arbeitgeber",
            "Acme GmbH",
            "Acme GmbH",
            "",
            "gross",
        ),
    ]
}

#[test]
fn facts_are_located_typed_and_keep_only_slots_their_value_can_hold() {
    let segments = seg(PAYSLIP);
    let typing = TypingContext::new(&segments, Some("payslip"), 2026);
    let facts = payslip_facts("Januar 2026");
    let (typed, inputs) = type_facts(&facts, &segments, &typing, me_core::doc_type("payslip"));
    assert_eq!(typed.len(), 4, "the unlocated fact is left out");
    assert_eq!(inputs.len(), typed.len());

    assert_eq!(inputs[0].label, "Gesamtbrutto");
    assert_eq!(inputs[0].slot.map(|s| s.key), Some("gross"));
    assert!(inputs[0].money);
    let gross = typed[0].candidate.as_ref().unwrap();
    assert_eq!(
        gross.value,
        CandidateValue::Money {
            amount: "4200.00".into(),
            currency: "EUR".into()
        }
    );
    assert_eq!(&PAYSLIP[typed[0].at.start..typed[0].at.end], "4.200,00 €");

    // A category that code could not map is kept for TypeSafe's category Choice.
    assert_eq!(inputs[2].slot.map(|s| s.key), Some("tax_class"));
    assert!(inputs[2].category_unmapped && typed[2].candidate.is_none());
    assert!(!inputs[2].money);

    assert!(inputs[3].slot.is_none() && !inputs[3].category_unmapped);
    assert_eq!(fact_label("person.tax_id"), "Tax ID");
    assert_eq!(fact_label("custom"), "custom");

    // Machine-readable-zone slots are filled by position, never from a reader tag.
    let passport = "Reisepass\nPassnummer C01X00T47\n";
    let segments = seg(passport);
    let typing = TypingContext::new(&segments, Some("passport"), 2026);
    let facts = vec![fact(
        "document.Passnummer",
        "C01X00T47",
        "Passnummer C01X00T47",
        "",
        "number",
    )];
    let (_, inputs) = type_facts(&facts, &segments, &typing, me_core::doc_type("passport"));
    assert!(inputs[0].slot.is_none());
}

#[test]
fn verdicts_become_stored_facts_linked_only_when_mapped() {
    let segments = seg(PAYSLIP);
    let typing = TypingContext::new(&segments, Some("payslip"), 2026);
    let facts = payslip_facts("Januar 2026");
    let (typed, inputs) = type_facts(&facts, &segments, &typing, me_core::doc_type("payslip"));

    let stored = read_fact(
        &typed[0],
        &inputs[0],
        &verdict(Owner::Anchor(SELF.into()), Band::Accept, 0.95, true),
        SELF,
    );
    assert_eq!(
        (stored.owner.as_str(), stored.owner_entity.as_deref()),
        ("self", Some(SELF))
    );
    assert_eq!(stored.state, FactState::Verified);
    assert_eq!(stored.slot.as_deref(), Some("gross"));
    assert_eq!(stored.period.as_deref(), Some("document"));
    assert_eq!(stored.context, "Januar 2026");
    assert_eq!(&PAYSLIP[stored.start..stored.end], "4.200,00 €");
    assert_eq!(stored.confidence, Some(0.95));

    let household = read_fact(
        &typed[1],
        &inputs[1],
        &verdict(Owner::Anchor(PARTNER.into()), Band::Check, 0.75, true),
        SELF,
    );
    assert_eq!(household.owner, "household");
    assert_eq!(household.owner_entity.as_deref(), Some(PARTNER));
    assert_eq!(household.state, FactState::Uncertain);
    assert_eq!(household.slot.as_deref(), Some("net"));

    // A rejected assumption keeps the grounded fact, detached from the profile.
    let mut rejected = verdict(
        Owner::Party("Erika Beispiel".into()),
        Band::Reject,
        0.3,
        false,
    );
    rejected.period = Some(PeriodKind::Cumulative);
    let party = read_fact(&typed[1], &inputs[1], &rejected, SELF);
    assert_eq!(
        (party.owner.as_str(), party.owner_name.as_deref()),
        ("party", Some("Erika Beispiel"))
    );
    assert_eq!(party.state, FactState::Uncertain);
    assert_eq!(party.slot, None);
    assert_eq!(party.period.as_deref(), Some("cumulative"));

    let mut organization = verdict(Owner::Organization, Band::Accept, 0.9, false);
    organization.period = Some(PeriodKind::NotPeriodic);
    let stored = read_fact(&typed[3], &inputs[3], &organization, SELF);
    assert_eq!(stored.owner, "organization");
    assert_eq!(stored.period.as_deref(), Some("none"));
    let mut unclear = verdict(Owner::Unclear, Band::Accept, 0.9, false);
    unclear.period = None;
    let stored = read_fact(&typed[2], &inputs[2], &unclear, SELF);
    assert_eq!((stored.owner.as_str(), stored.period), ("unclear", None));

    // Verification failed: kept on the document, unverified and unowned.
    let stored = unverified_fact(&typed[0], &inputs[0]);
    assert_eq!(stored.owner, "unknown");
    assert_eq!(stored.state, FactState::Unverified);
    assert_eq!((stored.slot, stored.confidence), (None, None));
    assert_eq!(&PAYSLIP[stored.start..stored.end], "4.200,00 €");
}

fn payslip_values(net_context: &str, extra: Option<ExtractedFact>) -> Vec<SlotValue> {
    let segments = seg(PAYSLIP);
    let typing = TypingContext::new(&segments, Some("payslip"), 2026);
    let mut facts = payslip_facts(net_context);
    facts.extend(extra);
    let (typed, inputs) = type_facts(&facts, &segments, &typing, me_core::doc_type("payslip"));
    let mut verdicts = vec![
        verdict(Owner::Anchor(SELF.into()), Band::Accept, 0.95, true),
        verdict(Owner::Anchor(SELF.into()), Band::Check, 0.75, true),
        verdict(Owner::Anchor(SELF.into()), Band::Accept, 0.9, true),
        verdict(Owner::Organization, Band::Accept, 0.9, false),
    ];
    verdicts[2].period = None;
    verdicts[2].category = Some("III".into());
    if typed.len() > verdicts.len() {
        let mut month = verdict(Owner::Anchor(SELF.into()), Band::Accept, 0.85, true);
        month.period = None;
        verdicts.push(month);
    }
    let candidates = find_candidates(&segments, &[], 2026);
    profile_values(
        me_core::doc_type("payslip").unwrap(),
        &candidates,
        &segments,
        &typed,
        &inputs,
        &verdicts,
    )
}

#[test]
fn verified_mappings_fill_the_profile_and_one_shared_period_is_the_pay_month() {
    let values = payslip_values("Januar 2026", None);
    let by_slot = |s: &str| values.iter().find(|v| v.slot == s);
    let gross = by_slot("gross").unwrap();
    assert!(!gross.check && gross.source == ConfidenceSource::Typesafe);
    assert!(
        matches!(&gross.content, SlotContent::Candidate(c) if c.value == CandidateValue::Money { amount: "4200.00".into(), currency: "EUR".into() })
    );
    assert!(
        by_slot("net").unwrap().check,
        "a check band asks a quick check"
    );
    assert!(matches!(
        &by_slot("tax_class").unwrap().content,
        SlotContent::Category { key } if key == "III"
    ));
    assert_eq!(values.iter().filter(|v| v.slot == "gross").count(), 1);

    let month = by_slot("pay_month").unwrap();
    let SlotContent::Candidate(c) = &month.content else {
        panic!("the pay month is a located period");
    };
    assert_eq!(
        c.value,
        CandidateValue::Period {
            start: "2026-01-01".into(),
            end: "2026-01-31".into()
        }
    );
    assert_eq!(&PAYSLIP[c.start..c.end], "Januar 2026");
    assert_eq!(month.confidence, 0.75, "the weakest contributing amount");

    // Two different periods: no pay month is guessed.
    let values = payslip_values("Dezember 2025", None);
    assert!(values.iter().all(|v| v.slot != "pay_month"));
    assert!(values.iter().any(|v| v.slot == "net"));

    // A verified pay-month fact wins over the fallback.
    let values = payslip_values(
        "Januar 2026",
        Some(fact(
            "document.Abrechnungsmonat",
            "Januar 2026",
            "Entgeltabrechnung Januar 2026",
            "",
            "pay_month",
        )),
    );
    let months: Vec<&SlotValue> = values.iter().filter(|v| v.slot == "pay_month").collect();
    assert_eq!(months.len(), 1);
    assert_eq!(months[0].confidence, 0.85);
}

#[test]
fn a_judged_refund_is_stored_negative_and_a_payment_positive() {
    let text = "Finanzamt Musterstadt\nBescheid 2025\nErstattung 1.234,56 €\n";
    let segments = seg(text);
    let typing = TypingContext::new(&segments, Some("tax_assessment"), 2026);
    let facts = vec![fact(
        "document.Erstattung",
        "1.234,56 €",
        "Erstattung 1.234,56 €",
        "",
        "tax_balance",
    )];
    let (typed, inputs) = type_facts(
        &facts,
        &segments,
        &typing,
        me_core::doc_type("tax_assessment"),
    );
    let amount = |refund: Option<bool>| {
        let mut v = verdict(Owner::Anchor(SELF.into()), Band::Accept, 0.9, true);
        v.refund = refund;
        let values = profile_values(
            me_core::doc_type("tax_assessment").unwrap(),
            &[],
            &segments,
            &typed,
            &inputs,
            &[v],
        );
        match &values.iter().find(|v| v.slot == "tax_balance")?.content {
            SlotContent::Candidate(c) => match &c.value {
                CandidateValue::Money { amount, .. } => Some(amount.clone()),
                _ => None,
            },
            SlotContent::Category { .. } => None,
        }
    };
    assert_eq!(amount(Some(true)).as_deref(), Some("-1234.56"));
    assert_eq!(amount(Some(false)).as_deref(), Some("1234.56"));

    let mut c = typed[0].candidate.clone().unwrap();
    c.value = CandidateValue::Money {
        amount: "-50.00".into(),
        currency: "EUR".into(),
    };
    apply_direction(&mut c, Some(false));
    assert!(matches!(&c.value, CandidateValue::Money { amount, .. } if amount == "50.00"));
    apply_direction(&mut c, None);
    assert!(matches!(&c.value, CandidateValue::Money { amount, .. } if amount == "50.00"));
    c.value = CandidateValue::Money {
        amount: "0.00".into(),
        currency: "EUR".into(),
    };
    apply_direction(&mut c, Some(true));
    assert!(matches!(&c.value, CandidateValue::Money { amount, .. } if amount == "0.00"));
}

fn mrz(valid: bool) -> Candidate {
    Candidate {
        id: "m0".into(),
        kind: CandidateKind::Mrz,
        text: "P<D<<MUSTERMANN<<MAX".into(),
        value: CandidateValue::Mrz(MrzData {
            format: "td3".into(),
            document_code: "P".into(),
            issuing_state: "D".into(),
            surname: "MUSTERMANN".into(),
            given_names: "MAX".into(),
            document_number: "C01X00T47".into(),
            nationality: "D".into(),
            birth_date: Some("1988-03-14".into()),
            sex: "M".into(),
            expiry_date: None,
            check_digits_valid: valid,
        }),
        segment_id: "s1".into(),
        start: 0,
        end: 20,
        line: "P<D<<MUSTERMANN<<MAX".into(),
        label: None,
        checksum: valid,
    }
}

#[test]
fn passport_values_come_from_the_machine_readable_zone_by_position() {
    let zone = mrz(true);
    assert_eq!(
        mrz_value(&zone, MrzField::DocumentNumber).map(|c| c.value),
        Some(CandidateValue::Identifier("C01X00T47".into()))
    );
    assert_eq!(
        mrz_value(&zone, MrzField::BirthDate).map(|c| (c.value, c.start, c.end)),
        Some((CandidateValue::Date("1988-03-14".into()), 0, 20))
    );
    assert!(mrz_value(&zone, MrzField::ExpiryDate).is_none());
    assert!(mrz_value(&mrz(false), MrzField::DocumentNumber).is_none());

    let segments = seg("P<D<<MUSTERMANN<<MAX\n");
    let passport = me_core::doc_type("passport").unwrap();
    let values = profile_values(passport, &[zone], &segments, &[], &[], &[]);
    let number = values.iter().find(|v| v.slot == "number").unwrap();
    assert_eq!(number.source, ConfidenceSource::Checksum);
    assert!(number.value_checked && !number.check);
    assert!(values.iter().any(|v| v.slot == "birth_date"));
    assert!(values.iter().all(|v| v.slot != "expires"));
    let values = profile_values(passport, &[mrz(false)], &segments, &[], &[], &[]);
    assert!(values.is_empty(), "invalid check digits establish nothing");
}

#[test]
fn only_accepted_assumptions_are_verified() {
    assert_eq!(fact_state(Band::Accept), FactState::Verified);
    assert_eq!(fact_state(Band::Check), FactState::Uncertain);
    assert_eq!(fact_state(Band::Reject), FactState::Uncertain);
}
