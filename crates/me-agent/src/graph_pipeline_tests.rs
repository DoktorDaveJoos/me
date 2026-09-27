//! Classify and reduce-side behavior with scripted TypeSafe answers. No
//! network, synthetic data only.
use super::*;
use crate::typesafe::DecisionResponse;
use me_core::{HouseholdMember, MrzData};
use std::collections::BTreeMap;

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
    fn score(mut self, key: &str, s: f64) -> Self {
        self.answers.insert(
            key.into(),
            json!({"type":"score","score":s,"confidence":0.9}),
        );
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
        self_entity: "00000000-0000-4000-8000-000000000001".into(),
        self_name: "Max Mustermann".into(),
        aliases: vec![],
        birth_date: None,
        household: vec![HouseholdMember {
            entity: "00000000-0000-4000-8000-000000000002".into(),
            name: "Lena Mustermann".into(),
            role: "partner".into(),
        }],
        setup_complete: true,
    }
}
fn cand(id: &str, kind: CandidateKind, text: &str, value: CandidateValue) -> Candidate {
    Candidate {
        id: id.into(),
        kind,
        text: text.into(),
        value,
        segment_id: "s1".into(),
        start: 0,
        end: text.len(),
        line: text.into(),
        label: None,
        checksum: false,
    }
}
const PAGE: &str = "Acme GmbH\nEntgeltabrechnung Januar 2026\nHerr Max Mustermann\nGesamtbrutto 4.200,00 €\nNetto 2.800,00 €\n";
fn segments() -> Vec<SourceSegment<'static>> {
    vec![SourceSegment {
        id: "s1",
        text: PAGE,
    }]
}
fn payslip_candidates() -> Vec<Candidate> {
    let money = |a: &str| CandidateValue::Money {
        amount: a.into(),
        currency: "EUR".into(),
    };
    vec![
        cand(
            "c0",
            CandidateKind::Organization,
            "Acme GmbH",
            CandidateValue::Text("Acme GmbH".into()),
        ),
        cand(
            "c1",
            CandidateKind::PersonName,
            "Max Mustermann",
            CandidateValue::Text("Max Mustermann".into()),
        ),
        cand("c2", CandidateKind::Money, "4.200,00 €", money("4200.00")),
        cand("c3", CandidateKind::Money, "2.800,00 €", money("2800.00")),
        cand(
            "c4",
            CandidateKind::Date,
            "01.01.2026",
            CandidateValue::Date("2026-01-01".into()),
        ),
        cand(
            "c5",
            CandidateKind::Date,
            "31.01.2026",
            CandidateValue::Date("2026-01-31".into()),
        ),
    ]
}

#[test]
fn classify_uses_one_fan_out_request_and_maps_the_subject_anchor() {
    let mut d = Scripted::default()
        .choice("family", "employment", 0.93)
        .choice("type_employment", "payslip", 0.91)
        .choice("subject", "self", 0.9)
        .noul("about_person", 0.95)
        .noul("readable", 0.99);
    let c = classify(
        Document {
            title: "payslip.pdf",
            segments: &segments(),
        },
        &anchors(),
        &payslip_candidates(),
        &mut d,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(d.requests.len(), 1);
    let questions = d.requests[0].1.as_object().unwrap();
    // Speculative type questions for every family with types, answered in the same call.
    assert!(questions.contains_key("type_identity") && questions.contains_key("type_tax"));
    assert!(
        questions
            .values()
            .all(|q| q["criteria"].as_object().is_none_or(|o| o.len() <= 255))
    );
    assert_eq!(c.doc_type.as_deref(), Some("payslip"));
    assert_eq!(c.subject.as_deref(), Some(anchors().self_entity.as_str()));
    assert_eq!(c.usage.models, vec!["jev-1.13.0".to_owned()]);
    assert!(c.eager_type().is_some());
}

#[test]
fn uncertain_types_keep_the_family_and_uncertain_families_become_other() {
    let run = |d: &mut Scripted| {
        classify(
            Document {
                title: "x.pdf",
                segments: &segments(),
            },
            &anchors(),
            &[],
            d,
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let mut d = Scripted::default()
        .choice("family", "insurance", 0.9)
        .choice("type_insurance", "insurance_policy", 0.55)
        .noul("readable", 0.9);
    let c = run(&mut d);
    assert_eq!((c.family.as_str(), c.doc_type), ("insurance", None));
    let mut d = Scripted::default()
        .choice("family", "tax", 0.3)
        .noul("readable", 0.9);
    assert_eq!(run(&mut d).family, "other");
    // Illegible or mixed files are never extracted, whatever the type answer.
    let mut d = Scripted::default()
        .choice("family", "tax", 0.95)
        .choice("type_tax", "tax_assessment", 0.95)
        .noul("readable", 0.1);
    assert_eq!(run(&mut d).doc_type, None);
    // A subject needs the existence Noul; the Choice alone always picks something.
    let mut d = Scripted::default()
        .choice("family", "tax", 0.95)
        .choice("subject", "household_0", 0.9)
        .noul("about_person", 0.2)
        .noul("readable", 0.9);
    assert_eq!(run(&mut d).subject, None);
}

#[test]
fn non_anchor_subjects_are_named_and_a_valid_mrz_overrides_the_judgment() {
    let mut cands = payslip_candidates();
    cands.push(cand(
        "c9",
        CandidateKind::PersonName,
        "Erika Musterfrau",
        CandidateValue::Text("Erika Musterfrau".into()),
    ));
    // Anchor names are filtered from the found names, so Erika is the first option.
    let mut d = Scripted::default()
        .choice("family", "employment", 0.9)
        .choice("subject", "named_0", 0.85)
        .noul("about_person", 0.9)
        .noul("readable", 0.9);
    let c = classify(
        Document {
            title: "x.pdf",
            segments: &segments(),
        },
        &anchors(),
        &cands,
        &mut d,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(c.subject_name.as_deref(), Some("Erika Musterfrau"));

    let mrz = |surname: &str, given: &str| MrzData {
        format: "td3".into(),
        document_code: "P".into(),
        issuing_state: "UTO".into(),
        surname: surname.into(),
        given_names: given.into(),
        document_number: "C01X00T47".into(),
        nationality: "UTO".into(),
        birth_date: Some("1988-03-14".into()),
        sex: "F".into(),
        expiry_date: Some("2031-04-30".into()),
        check_digits_valid: true,
    };
    let mut d = Scripted::default()
        .choice("family", "identity", 0.95)
        .choice("type_identity", "passport", 0.95)
        .choice("subject", "self", 0.9)
        .noul("about_person", 0.9)
        .noul("readable", 0.9);
    let passport = vec![cand(
        "m0",
        CandidateKind::Mrz,
        "P<UTOMUSTERMANN<<LENA",
        CandidateValue::Mrz(mrz("MUSTERMANN", "LENA")),
    )];
    let c = classify(
        Document {
            title: "pass.jpg",
            segments: &segments(),
        },
        &anchors(),
        &passport,
        &mut d,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        c.subject.as_deref(),
        Some("00000000-0000-4000-8000-000000000002")
    );
    assert_eq!(c.subject_confidence, 1.);
}

#[test]
fn reduce_checks_batch_pairs_and_never_merge_on_their_own() {
    let conflicts = vec![
        ValueConflict {
            first: "a1".into(),
            first_value: "Musterstr. 1".into(),
            second: "a2".into(),
            second_value: "Musterstraße 1".into(),
            label: "Street".into(),
        },
        ValueConflict {
            first: "b1".into(),
            first_value: "Berlin".into(),
            second: "b2".into(),
            second_value: "Hamburg".into(),
            label: "City".into(),
        },
    ];
    let mut d = Scripted::default()
        .choice("p0::relation", "same", 0.93)
        .choice("p1::relation", "different", 0.99);
    let (same, usage) = judge_equivalence(&conflicts, &mut d, &AtomicBool::new(false)).unwrap();
    assert_eq!(same, vec![("a1".to_owned(), "a2".to_owned())]);
    assert_eq!(usage.requests, 1);

    let pairs = vec![
        MergeCandidate {
            a: "o1".into(),
            a_label: "Techniker Krankenkasse".into(),
            b: "o2".into(),
            b_label: "Techniker KK".into(),
            kind: "organization".into(),
        },
        MergeCandidate {
            a: "o3".into(),
            a_label: "Acme Bank".into(),
            b: "o4".into(),
            b_label: "Acme Versicherung".into(),
            kind: "organization".into(),
        },
    ];
    let mut d = Scripted::default()
        .score("p0::link_state", 1.9)
        .score("p1::link_state", 0.2);
    let (judged, _) = align_organizations(&pairs, &mut d, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        judged.iter().map(|j| j.propose).collect::<Vec<_>>(),
        vec![true, false]
    );

    let docs: Vec<(String, String, String)> = (0..50)
        .map(|i| {
            (
                format!("s{i}"),
                format!("Letter {i}"),
                "Synthetic text ".repeat(20),
            )
        })
        .collect();
    let mut d = Scripted::default().noul("doc_3", 0.92);
    let (found, usage) = search_documents(
        "When does my contract end?",
        &docs,
        &mut d,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(usage.requests, 2, "40 documents per request at most");
    assert!(found.contains(&"s3".to_owned()));
}

#[test]
fn every_question_and_threshold_stays_within_documented_limits() {
    let subjects = q::subject_options(
        &anchors(),
        &(0..40).map(|i| format!("Person {i}")).collect::<Vec<_>>(),
    );
    let questions = q::classify_questions(&subjects);
    for (key, question) in questions.as_object().unwrap() {
        if let Some(options) = question["criteria"].as_object() {
            assert!(options.len() <= 255, "{key}");
        }
    }
    assert!(subjects.named.len() <= 24);
    assert_eq!(q::MODEL, crate::typesafe::MODEL);
}

#[test]
#[ignore = "One live TypeSafe request on synthetic text; needs private API configuration"]
fn live_synthetic_payslip_is_classified() {
    let mut typesafe = crate::typesafe::TypeSafe::configured().unwrap();
    let cancel = AtomicBool::new(false);
    let doc = Document {
        title: "Entgeltabrechnung_2026_01.pdf",
        segments: &segments(),
    };
    let c = classify(
        doc,
        &anchors(),
        &payslip_candidates(),
        &mut typesafe,
        &cancel,
    )
    .unwrap();
    assert_eq!(c.family, "employment", "{c:?}");
    assert_eq!(c.doc_type.as_deref(), Some("payslip"), "{c:?}");
    assert_eq!(
        c.subject.as_deref(),
        Some(anchors().self_entity.as_str()),
        "{c:?}"
    );
    assert!(c.usage.models.iter().all(|m| m.starts_with("jev-1.13")));
    assert!(c.eager_type().is_some());
}
