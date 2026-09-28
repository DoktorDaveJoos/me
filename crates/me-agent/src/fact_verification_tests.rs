//! Verification of reader assumptions with scripted TypeSafe answers. No network,
//! synthetic data only.
use super::*;
use crate::typesafe::{DecisionResponse, Decisions, Result};
use me_core::{HouseholdMember, IdentityAnchors, ImportErrorKind};
use serde_json::{Map, Value, json};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};

/// Answers questions by key; unscripted Choices pick `none`/`other` (or the first
/// option), unscripted Nouls answer 0 and Scores 0.
#[derive(Default)]
struct Scripted {
    answers: BTreeMap<String, Value>,
    requests: Vec<(Value, Value)>,
}
impl Scripted {
    fn choice(mut self, key: &str, choice: &str, confidence: f64) -> Self {
        self.answers.insert(
            key.into(),
            json!({"type":"choice","choice":choice,"confidence":confidence}),
        );
        self
    }
    fn noul(mut self, key: &str, p: f64) -> Self {
        self.answers
            .insert(key.into(), json!({"type":"noul","noul":p}));
        self
    }
    /// Confident owner (the subject), document period and mapping for `f`.
    fn sure(self, f: &str) -> Self {
        self.choice(&format!("{f}::owner"), "self", 0.95)
            .choice(&format!("{f}::period"), "document", 0.95)
            .noul(&format!("{f}::mapping"), 0.95)
    }
}
impl Decisions for Scripted {
    fn evaluate(
        &mut self,
        state: Value,
        questions: Value,
        _: &AtomicBool,
    ) -> Result<DecisionResponse> {
        self.requests.push((state, questions.clone()));
        let mut out = Map::new();
        for (key, question) in questions.as_object().unwrap() {
            let answer =
                self.answers
                    .get(key)
                    .cloned()
                    .unwrap_or_else(|| match question["type"].as_str() {
                        Some("choice") => {
                            let options = question["criteria"].as_object().unwrap();
                            let pick = ["none", "other"]
                                .into_iter()
                                .find(|k| options.contains_key(*k))
                                .map(str::to_owned)
                                .unwrap_or_else(|| options.keys().next().unwrap().clone());
                            json!({"type":"choice","choice":pick,"confidence":1.0})
                        }
                        Some("score") => json!({"type":"score","score":0.0,"confidence":1.0}),
                        _ => json!({"type":"noul","noul":0.0}),
                    });
            out.insert(key.clone(), answer);
        }
        Ok(DecisionResponse {
            answers: Value::Object(out),
            model: Some("jev-1.13.0".into()),
            input_tokens: Some(100),
            output_tokens: Some(0),
        })
    }
}

/// Synthetic letterhead of a payslip.
const LETTERHEAD: &str = "Muster GmbH · Musterweg 1 · 12345 Musterstadt\nEntgeltabrechnung Januar 2026\nHerrn Max Beispiel";

fn anchors() -> IdentityAnchors {
    IdentityAnchors {
        self_entity: "me".into(),
        self_name: "Max Beispiel".into(),
        aliases: vec![],
        birth_date: None,
        household: Vec::<HouseholdMember>::new(),
        setup_complete: true,
    }
}
fn payslip_fact(slot: &str, value: &str) -> FactInput {
    FactInput {
        label: "Lohnsteuer".into(),
        value: value.into(),
        quote: format!("Lohnsteuer   {value}"),
        line: format!("Lohnsteuer   {value}"),
        money: true,
        slot: me_core::doc_type("payslip").unwrap().slot(slot),
        category_unmapped: false,
    }
}
fn balance_fact() -> FactInput {
    FactInput {
        slot: me_core::doc_type("tax_assessment")
            .unwrap()
            .slot("tax_balance"),
        label: "Erstattung".into(),
        ..payslip_fact("gross", "812,00")
    }
}
fn verify(
    d: &mut impl Decisions,
    facts: &[FactInput],
    anchors: &IdentityAnchors,
    named: &[String],
) -> Result<Verification> {
    verify_facts(
        "payslip.pdf",
        "Payslip",
        LETTERHEAD,
        facts,
        anchors,
        named,
        Some("me"),
        0.8,
        d,
        &AtomicBool::new(false),
    )
}
fn run(d: &mut Scripted, facts: &[FactInput]) -> Verification {
    verify(d, facts, &anchors(), &[]).unwrap()
}
/// The error kind of a verification that must fail.
fn failure_kind(d: &mut impl Decisions, facts: &[FactInput]) -> ImportErrorKind {
    verify(d, facts, &anchors(), &[])
        .err()
        .expect("the response must be rejected")
        .kind
}

#[test]
fn confident_assumptions_are_accepted_silently() {
    let mut d = Scripted::default()
        .choice("f0::owner", "self", 0.95)
        .choice("f0::period", "document", 0.93)
        .noul("f0::mapping", 0.92);
    let v = run(&mut d, &[payslip_fact("wage_tax", "1.032,58")]);
    assert_eq!(v.verdicts[0].band, Band::Accept);
    assert!(v.verdicts[0].mapped);
    assert_eq!(v.verdicts[0].owner, Owner::Anchor("me".into()));
}

#[test]
fn a_doubtful_assumption_becomes_a_check_and_a_wrong_one_is_rejected() {
    let mut d = Scripted::default()
        .choice("f0::owner", "self", 0.72)
        .choice("f0::period", "document", 0.9)
        .noul("f0::mapping", 0.9)
        .choice("f1::owner", "self", 0.95)
        .choice("f1::period", "document", 0.9)
        .noul("f1::mapping", 0.9)
        .noul("f1::off_target", 0.8);
    let v = run(
        &mut d,
        &[
            payslip_fact("wage_tax", "1.032,58"),
            payslip_fact("church_tax", "0,00"),
        ],
    );
    assert_eq!(v.verdicts[0].band, Band::Check);
    assert!(v.verdicts[0].mapped);
    assert_eq!(v.verdicts[1].band, Band::Reject);
    assert!(!v.verdicts[1].mapped);
}

#[test]
fn a_year_to_date_total_or_another_persons_value_never_enters_the_monthly_slot() {
    let mut d = Scripted::default()
        .choice("f0::owner", "self", 0.95)
        .choice("f0::period", "cumulative", 0.95)
        .noul("f0::mapping", 0.9)
        .choice("f1::owner", "organization", 0.95)
        .choice("f1::period", "document", 0.95)
        .noul("f1::mapping", 0.9);
    let v = run(
        &mut d,
        &[
            payslip_fact("wage_tax", "12.390,96"),
            payslip_fact("gross", "5.340,00"),
        ],
    );
    assert!(!v.verdicts[0].mapped && !v.verdicts[1].mapped);
    assert_eq!(
        v.verdicts[0].band,
        Band::Accept,
        "the fact itself stays a verified document fact"
    );
}

#[test]
fn facts_are_verified_in_batches_of_twenty() {
    let facts: Vec<FactInput> = (0..45)
        .map(|i| FactInput {
            slot: None,
            ..payslip_fact("wage_tax", &format!("{i},00"))
        })
        .collect();
    let mut d = Scripted::default();
    let v = run(&mut d, &facts);
    assert_eq!(d.requests.len(), 3);
    assert_eq!(v.verdicts.len(), 45);
    assert!(
        d.requests[0]
            .1
            .as_object()
            .unwrap()
            .contains_key("document::correction")
    );
    assert!(
        !d.requests[1]
            .1
            .as_object()
            .unwrap()
            .contains_key("document::correction")
    );
}

#[test]
fn competing_mappings_are_decided_by_a_choice_over_the_readers_facts() {
    let mut d = Scripted::default()
        .choice("f0::owner", "self", 0.95)
        .choice("f0::period", "document", 0.95)
        .noul("f0::mapping", 0.9)
        .choice("f1::owner", "self", 0.95)
        .choice("f1::period", "document", 0.95)
        .noul("f1::mapping", 0.9)
        .choice("slot_net", "f1", 0.9);
    let v = run(
        &mut d,
        &[
            payslip_fact("net", "3.210,05"),
            payslip_fact("net", "3.210,05 "),
        ],
    );
    assert_eq!(d.requests.len(), 2);
    assert!(!v.verdicts[0].mapped && v.verdicts[1].mapped);
}

#[test]
fn a_balance_direction_and_a_correction_are_judged_not_computed() {
    let mut d = Scripted::default()
        .choice("f0::owner", "self", 0.95)
        .choice("f0::period", "not_periodic", 0.9)
        .noul("f0::mapping", 0.9)
        .choice("f0::direction", "refund", 0.93)
        .noul("document::correction", 0.9);
    let v = verify_facts(
        "bescheid.pdf",
        "Tax assessment",
        "Finanzamt Musterstadt\nBescheid für 2025 über Einkommensteuer",
        &[balance_fact()],
        &anchors(),
        &[],
        Some("me"),
        0.8,
        &mut d,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(v.verdicts[0].refund, Some(true));
    assert!(v.verdicts[0].mapped);
    assert!(v.correction);
}

#[test]
fn bands_follow_the_documented_thresholds() {
    assert_eq!(band(0.95, 0.1, 0.8), Band::Accept);
    assert_eq!(band(0.75, 0.1, 0.8), Band::Check);
    assert_eq!(band(0.55, 0.1, 0.8), Band::Reject);
    assert_eq!(band(0.95, 0.71, 0.8), Band::Reject);
    // Boundaries: a failure of exactly VERIFY_FIRE does not reject, a confidence of
    // exactly SLOT_FLOOR is a check, and exactly the threshold accepts.
    assert_eq!(band(0.95, q::VERIFY_FIRE, 0.8), Band::Accept);
    assert_eq!(band(q::SLOT_FLOOR, 0.1, 0.8), Band::Check);
    assert_eq!(band(0.8, 0.1, 0.8), Band::Accept);
}

fn policy_fact(slot: &str, money: bool, category_unmapped: bool) -> FactInput {
    FactInput {
        slot: me_core::doc_type("insurance_policy").unwrap().slot(slot),
        money,
        category_unmapped,
        ..payslip_fact("wage_tax", "84,20")
    }
}

#[test]
fn each_fact_is_asked_only_what_its_value_type_needs() {
    let owners = q::owner_options(&anchors(), &["Erika Beispiel".into()]);
    assert!(!owners.criteria.contains_key("none"));
    for key in ["self", "named_0", "organization", "unclear"] {
        assert!(owners.criteria.contains_key(key), "{key}");
    }
    let facts = [
        FactInput {
            money: false,
            slot: None,
            ..payslip_fact("wage_tax", "Steuerklasse 1")
        },
        policy_fact("premium", true, false),
        policy_fact("kind", false, false),
        policy_fact("kind", false, true),
        balance_fact(),
    ];
    let batch: Vec<(usize, &FactInput)> = facts.iter().enumerate().collect();
    let questions = q::fact_questions(&batch, &owners, false);
    let questions = questions.as_object().unwrap();
    let asked = |f: &str| -> Vec<&str> {
        let prefix = format!("{f}::");
        let mut heads: Vec<&str> = questions
            .keys()
            .filter_map(|k| k.strip_prefix(prefix.as_str()))
            .collect();
        heads.sort_unstable();
        heads
    };
    let with = |extra: &[&'static str]| {
        let mut all = vec!["hallucinated", "off_target", "owner"];
        all.extend_from_slice(extra);
        all.sort_unstable();
        all
    };
    assert_eq!(asked("f0"), with(&[]));
    assert_eq!(asked("f1"), with(&["mapping", "payment_period", "period"]));
    assert_eq!(asked("f2"), with(&["mapping"]));
    assert_eq!(asked("f3"), with(&["category", "mapping"]));
    assert_eq!(asked("f4"), with(&["direction", "mapping", "period"]));
    assert!(!questions.contains_key("document::correction"));
    assert!(
        questions["f3::category"]["criteria"]
            .get("health")
            .is_some()
    );
    assert!(
        questions["f1::payment_period"]["criteria"]
            .get("month")
            .is_some()
    );
    assert!(questions["f0::owner"]["criteria"].get("none").is_none());
}

#[test]
fn role_slots_take_judged_payment_periods_categories_and_named_owners() {
    let facts = [
        policy_fact("premium", true, false),
        policy_fact("kind", false, true),
        policy_fact("kind", false, true),
    ];
    let mut d = Scripted::default()
        .choice("f0::owner", "organization", 0.95)
        .choice("f0::period", "document", 0.9)
        .noul("f0::mapping", 0.9)
        .choice("f0::payment_period", "month", 0.9)
        .choice("f1::owner", "named_0", 0.9)
        .noul("f1::mapping", 0.9)
        .choice("f1::category", "health", 0.85)
        .choice("f2::owner", "organization", 0.9)
        .noul("f2::mapping", 0.9)
        .choice("f2::category", "other", 0.9);
    let v = verify_facts(
        "police.pdf",
        "Insurance policy",
        "Muster Versicherung AG\nVersicherungsschein",
        &facts,
        &anchors(),
        &["Erika Beispiel".into()],
        Some("me"),
        0.8,
        &mut d,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(v.verdicts[0].mapped);
    assert_eq!(v.verdicts[0].payment_period, Some(me_core::Period::Month));
    assert_eq!(v.verdicts[1].owner, Owner::Party("Erika Beispiel".into()));
    assert_eq!(v.verdicts[1].category.as_deref(), Some("health"));
    assert!(v.verdicts[1].mapped && v.verdicts[1].period.is_none());
    assert_eq!(v.verdicts[2].category, None);
    assert!(
        !v.verdicts[2].mapped,
        "an unmapped category needs a judged option"
    );
    assert_eq!(
        d.requests.len(),
        1,
        "an unmapped fact does not compete for its slot"
    );
}

/// Drops one answer from every response, as a malformed TypeSafe reply would.
struct Dropping(Scripted, &'static str);
impl Decisions for Dropping {
    fn evaluate(
        &mut self,
        state: Value,
        questions: Value,
        cancel: &AtomicBool,
    ) -> Result<DecisionResponse> {
        let mut response = self.0.evaluate(state, questions, cancel)?;
        response.answers.as_object_mut().unwrap().remove(self.1);
        Ok(response)
    }
}

#[test]
fn a_missing_failure_head_fails_closed_instead_of_accepting() {
    let mut d = Dropping(Scripted::default().sure("f0"), "f0::hallucinated");
    assert_eq!(
        failure_kind(&mut d, &[payslip_fact("wage_tax", "1.032,58")]),
        ImportErrorKind::InvalidOutput
    );
}

/// Every string in a question: instructions and option descriptions.
fn texts(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) => out.push(s.clone()),
        Value::Array(items) => items.iter().for_each(|v| texts(v, out)),
        Value::Object(map) => map.values().for_each(|v| texts(v, out)),
        _ => {}
    }
}

#[test]
fn every_question_is_answerable_from_the_state_it_is_sent_with() {
    let facts = [
        payslip_fact("wage_tax", "1.032,58"),
        balance_fact(),
        policy_fact("premium", true, false),
        policy_fact("kind", false, true),
        payslip_fact("net", "3.210,05"),
        payslip_fact("net", "3.210,05 "),
    ];
    let mut d = Scripted::default()
        .sure("f4")
        .sure("f5")
        .choice("slot_net", "f5", 0.9);
    run(&mut d, &facts);
    assert_eq!(d.requests.len(), 2, "one fact request and one competing");
    for (state, questions) in &d.requests {
        assert_eq!(state["document"]["excerpt"], LETTERHEAD);
        assert_eq!(state["document"]["file_name"], "payslip.pdf");
        assert_eq!(state["document"]["type"], "Payslip");
        for (key, question) in questions.as_object().unwrap() {
            let mut all = Vec::new();
            texts(question, &mut all);
            for text in all {
                for reference in text.split('`').skip(1).step_by(2) {
                    assert_ne!(
                        reference, "document",
                        "{key}: the document itself is not in the state"
                    );
                    let resolved = reference
                        .split('.')
                        .try_fold(state, |node, part| node.get(part));
                    assert!(
                        resolved.is_some(),
                        "{key}: `{reference}` is not in the state"
                    );
                }
            }
        }
    }
}

#[test]
fn only_the_letterhead_of_the_excerpt_is_sent() {
    let text: String = (1..=20).map(|i| format!("Zeile {i}\n")).collect();
    let mut d = Scripted::default();
    verify_facts(
        "payslip.pdf",
        "Payslip",
        &text,
        &[payslip_fact("wage_tax", "1.032,58")],
        &anchors(),
        &[],
        Some("me"),
        0.8,
        &mut d,
        &AtomicBool::new(false),
    )
    .unwrap();
    let excerpt = d.requests[0].0["document"]["excerpt"].as_str().unwrap();
    assert_eq!(excerpt.lines().count(), q::LETTERHEAD_LINES);
    assert!(excerpt.ends_with("Zeile 12"));
    // A cut inside a multi-byte character moves back to its start.
    let wide = format!("x{}", "ä".repeat(q::LETTERHEAD_BYTES));
    let head = letterhead(&wide);
    assert!(head.len() <= q::LETTERHEAD_BYTES && head.len() > q::LETTERHEAD_BYTES - 2);
    assert!(wide.starts_with(head));
}

#[test]
fn a_named_person_who_is_an_anchor_is_offered_only_as_the_anchor() {
    let owners = q::owner_options(&anchors(), &["Max Beispiel".into(), "Erika Muster".into()]);
    assert!(owners.criteria.contains_key("self"));
    assert_eq!(
        owners.named,
        vec![("named_0".to_owned(), "Erika Muster".to_owned())]
    );
    let named = owners
        .criteria
        .keys()
        .filter(|k| k.starts_with("named_"))
        .count();
    assert_eq!(named, 1);
    assert_eq!(owners.criteria["named_0"], "Erika Muster");
    // Spelling variants of an anchor or of one person are not separate options.
    let owners = q::owner_options(
        &anchors(),
        &[
            "MAX  BEISPIEL".into(),
            "Erika Muster".into(),
            "erika muster".into(),
        ],
    );
    assert_eq!(owners.named.len(), 1);
}

#[test]
fn a_doubtful_direction_payment_period_or_category_becomes_a_check() {
    let facts = [
        balance_fact(),
        policy_fact("premium", true, false),
        policy_fact("kind", false, true),
    ];
    let mut d = Scripted::default()
        .choice("f0::owner", "self", 0.95)
        .choice("f0::period", "not_periodic", 0.95)
        .noul("f0::mapping", 0.95)
        .choice("f0::direction", "refund", 0.7)
        .choice("f1::owner", "organization", 0.95)
        .choice("f1::period", "document", 0.95)
        .noul("f1::mapping", 0.95)
        .choice("f1::payment_period", "month", 0.7)
        .choice("f2::owner", "organization", 0.95)
        .noul("f2::mapping", 0.95)
        .choice("f2::category", "health", 0.7);
    let v = run(&mut d, &facts);
    for (i, verdict) in v.verdicts.iter().enumerate() {
        assert_eq!(verdict.confidence, 0.7, "f{i}");
        assert_eq!(verdict.band, Band::Check, "f{i}");
        assert!(verdict.mapped, "f{i}");
    }
}

#[test]
fn a_doubtful_competing_choice_makes_the_winner_a_check() {
    let mut d = Scripted::default()
        .sure("f0")
        .sure("f1")
        .choice("slot_net", "f1", 0.7);
    let v = run(
        &mut d,
        &[
            payslip_fact("net", "3.210,05"),
            payslip_fact("net", "3.210,05 "),
        ],
    );
    assert!(!v.verdicts[0].mapped && v.verdicts[1].mapped);
    assert_eq!(v.verdicts[1].confidence, 0.7);
    assert_eq!(v.verdicts[1].band, Band::Check);
    assert_eq!(v.verdicts[0].band, Band::Accept);
}

#[test]
fn every_asked_question_needs_a_recognised_answer() {
    let wage_tax = || [payslip_fact("wage_tax", "1.032,58")];
    for dropped in ["f0::period", "f0::mapping", "document::correction"] {
        let mut d = Dropping(Scripted::default().sure("f0"), dropped);
        assert_eq!(
            failure_kind(&mut d, &wage_tax()),
            ImportErrorKind::InvalidOutput,
            "{dropped}"
        );
    }
    let mut d = Dropping(Scripted::default().sure("f0"), "f0::direction");
    assert_eq!(
        failure_kind(&mut d, &[balance_fact()]),
        ImportErrorKind::InvalidOutput
    );
    let unknown = [
        ("f0::owner", "stranger", Vec::from(wage_tax())),
        ("f0::period", "weekly", Vec::from(wage_tax())),
        ("f0::direction", "maybe", vec![balance_fact()]),
        (
            "f0::payment_period",
            "fortnight",
            vec![policy_fact("premium", true, false)],
        ),
        (
            "f0::category",
            "pets",
            vec![policy_fact("kind", false, true)],
        ),
    ];
    for (key, choice, facts) in unknown {
        let mut d = Scripted::default().sure("f0").choice(key, choice, 0.95);
        assert_eq!(
            failure_kind(&mut d, &facts),
            ImportErrorKind::InvalidOutput,
            "{key} = {choice}"
        );
    }
    let nets = [
        payslip_fact("net", "3.210,05"),
        payslip_fact("net", "3.210,05 "),
    ];
    let mut d = Scripted::default()
        .sure("f0")
        .sure("f1")
        .choice("slot_net", "f7", 0.95);
    assert_eq!(failure_kind(&mut d, &nets), ImportErrorKind::InvalidOutput);
    let mut d = Dropping(Scripted::default().sure("f0").sure("f1"), "slot_net");
    assert_eq!(failure_kind(&mut d, &nets), ImportErrorKind::InvalidOutput);
}

#[test]
fn a_competing_choice_offers_at_most_sixty_facts() {
    let facts: Vec<FactInput> = (0..62)
        .map(|i| payslip_fact("net", &format!("3.210,{i:02}")))
        .collect();
    let mut d = (0..62).fold(Scripted::default(), |d, i| d.sure(&format!("f{i}")));
    d = d.choice("slot_net", "f0", 0.95);
    let v = run(&mut d, &facts);
    assert_eq!(d.requests.len(), 5, "four fact batches and one competing");
    let (state, questions) = d.requests.last().unwrap();
    let options = questions["slot_net"]["criteria"].as_object().unwrap();
    assert_eq!(
        options.len(),
        q::MAX_SLOT_OPTIONS + 1,
        "sixty facts and none"
    );
    assert!(options.contains_key("f59") && !options.contains_key("f60"));
    assert_eq!(
        state["facts"].as_object().unwrap().len(),
        q::MAX_SLOT_OPTIONS
    );
    assert!(v.verdicts[0].mapped);
    assert!(v.verdicts[1..].iter().all(|v| !v.mapped));
}

#[test]
fn a_household_members_value_never_fills_the_subjects_slot() {
    let anchors = IdentityAnchors {
        household: vec![HouseholdMember {
            entity: "partner".into(),
            name: "Erika Beispiel".into(),
            role: "partner".into(),
        }],
        ..anchors()
    };
    let mut d = Scripted::default()
        .sure("f0")
        .choice("f0::owner", "household_0", 0.95);
    let v = verify(
        &mut d,
        &[payslip_fact("wage_tax", "1.032,58")],
        &anchors,
        &[],
    )
    .unwrap();
    assert_eq!(v.verdicts[0].owner, Owner::Anchor("partner".into()));
    assert!(!v.verdicts[0].mapped);
    assert_eq!(v.verdicts[0].band, Band::Accept);
}

#[test]
fn an_amount_of_another_period_never_fills_a_periodic_slot() {
    let mut d = Scripted::default()
        .sure("f0")
        .choice("f0::period", "other", 0.95);
    let v = run(&mut d, &[payslip_fact("wage_tax", "1.032,58")]);
    assert_eq!(v.verdicts[0].period, Some(PeriodKind::Other));
    assert!(!v.verdicts[0].mapped);
}

#[test]
fn a_balance_without_a_judged_direction_is_not_mapped() {
    let mut d = Scripted::default()
        .sure("f0")
        .choice("f0::period", "not_periodic", 0.95)
        .choice("f0::direction", "unclear", 0.95);
    let v = run(&mut d, &[balance_fact()]);
    assert_eq!(v.verdicts[0].refund, None);
    assert!(!v.verdicts[0].mapped);
}

#[test]
fn a_competing_choice_of_none_or_below_the_floor_maps_no_fact() {
    let nets = || {
        [
            payslip_fact("net", "3.210,05"),
            payslip_fact("net", "3.210,05 "),
        ]
    };
    for (winner, confidence) in [("none", 0.95), ("f1", 0.5)] {
        let mut d = Scripted::default()
            .sure("f0")
            .sure("f1")
            .choice("slot_net", winner, confidence);
        let v = run(&mut d, &nets());
        assert!(
            v.verdicts.iter().all(|v| !v.mapped),
            "{winner} at {confidence}"
        );
    }
}

#[test]
fn answers_of_a_later_batch_reach_their_own_fact() {
    let facts: Vec<FactInput> = (0..30)
        .map(|i| FactInput {
            slot: None,
            ..payslip_fact("wage_tax", &format!("{i},00"))
        })
        .collect();
    let mut d = Scripted::default()
        .choice("f5::owner", "self", 0.95)
        .choice("f25::owner", "organization", 0.95)
        .noul("f25::off_target", 0.9);
    let v = run(&mut d, &facts);
    assert!(d.requests[1].1.get("f25::off_target").is_some());
    assert!(d.requests[1].0["facts"].get("f25").is_some());
    assert_eq!(v.verdicts[25].owner, Owner::Organization);
    assert_eq!(v.verdicts[25].failure, 0.9);
    assert_eq!(v.verdicts[25].band, Band::Reject);
    assert_eq!(v.verdicts[5].owner, Owner::Anchor("me".into()));
    for (i, verdict) in v.verdicts.iter().enumerate().filter(|(i, _)| *i != 25) {
        assert_eq!(verdict.band, Band::Accept, "f{i}");
    }
}
