//! Verification of reader assumptions with scripted TypeSafe answers. No network,
//! synthetic data only.
use super::*;
use crate::typesafe::{DecisionResponse, Decisions, Result};
use me_core::{HouseholdMember, IdentityAnchors};
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
fn run(d: &mut Scripted, facts: &[FactInput]) -> Verification {
    verify_facts(
        "payslip.pdf",
        "Payslip",
        facts,
        &anchors(),
        &[],
        Some("me"),
        0.8,
        d,
        &AtomicBool::new(false),
    )
    .unwrap()
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
    let slot = me_core::doc_type("tax_assessment")
        .unwrap()
        .slot("tax_balance");
    let fact = FactInput {
        slot,
        label: "Erstattung".into(),
        ..payslip_fact("gross", "812,00")
    };
    let mut d = Scripted::default()
        .choice("f0::owner", "self", 0.95)
        .choice("f0::period", "not_periodic", 0.9)
        .noul("f0::mapping", 0.9)
        .choice("f0::direction", "refund", 0.93)
        .noul("document::correction", 0.9);
    let v = verify_facts(
        "bescheid.pdf",
        "Tax assessment",
        &[fact],
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
    let balance = FactInput {
        slot: me_core::doc_type("tax_assessment")
            .unwrap()
            .slot("tax_balance"),
        ..payslip_fact("wage_tax", "812,00")
    };
    let facts = [
        FactInput {
            money: false,
            slot: None,
            ..payslip_fact("wage_tax", "Steuerklasse 1")
        },
        policy_fact("premium", true, false),
        policy_fact("kind", false, false),
        policy_fact("kind", false, true),
        balance,
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
    let mut d = Dropping(
        Scripted::default()
            .choice("f0::owner", "self", 0.95)
            .choice("f0::period", "document", 0.95)
            .noul("f0::mapping", 0.95),
        "f0::hallucinated",
    );
    let result = verify_facts(
        "payslip.pdf",
        "Payslip",
        &[payslip_fact("wage_tax", "1.032,58")],
        &anchors(),
        &[],
        Some("me"),
        0.8,
        &mut d,
        &AtomicBool::new(false),
    );
    assert!(result.is_err());
}
