//! Classify/Extract/verification behavior with scripted TypeSafe answers. No
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

fn payslip_classification() -> Classification {
    Classification {
        family: "employment".into(),
        family_confidence: 0.95,
        doc_type: Some("payslip".into()),
        type_confidence: Some(0.95),
        subject: Some(anchors().self_entity),
        subject_confidence: 0.9,
        subject_name: None,
        readable: 1.,
        mixed: 0.,
        usage: StageUsage::default(),
    }
}

#[test]
fn extract_selects_candidates_and_routes_uncertain_values_to_quick_checks() {
    let kind = me_core::doc_type("payslip").unwrap();
    let mut d = Scripted::default()
        .choice("slot_employer", "c0", 0.97)
        .noul("present_employer", 0.95)
        .choice("slot_gross", "c2", 0.72)
        .noul("present_gross", 0.9)
        .choice("slot_net", "c3", 0.95)
        .noul("present_net", 0.2)
        .choice("slot_period_start", "c4", 0.95)
        .noul("present_period_start", 0.9)
        .choice("slot_period_end", "c5", 0.9)
        .noul("present_period_end", 0.5);
    let e = extract(
        Document {
            title: "payslip.pdf",
            segments: &segments(),
        },
        kind,
        &payslip_classification(),
        &payslip_candidates(),
        0.8,
        &mut d,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(d.requests.len(), 1);
    let (state, questions) = &d.requests[0];
    // Only candidate kinds a slot accepts are offered, each with `none`.
    let gross = questions["slot_gross"]["criteria"].as_object().unwrap();
    assert_eq!(
        gross.keys().cloned().collect::<Vec<_>>(),
        vec!["c2", "c3", "none"]
    );
    assert!(
        state["document"]["lines"]
            .as_array()
            .unwrap()
            .iter()
            .all(|l| l.as_str().unwrap().starts_with('L'))
    );
    let by_slot = |s: &str| e.graph.values.iter().find(|v| v.slot == s);
    assert!(!by_slot("employer").unwrap().check);
    assert!(
        by_slot("gross").unwrap().check,
        "confidence below the vault threshold"
    );
    assert!(by_slot("net").is_none(), "existence Noul says absent");
    assert!(
        by_slot("period_end").unwrap().check,
        "existence Noul is only partial"
    );
    assert!(e.missing_required.is_empty());

    let mut d = Scripted::default();
    let e = extract(
        Document {
            title: "payslip.pdf",
            segments: &segments(),
        },
        kind,
        &payslip_classification(),
        &payslip_candidates(),
        0.8,
        &mut d,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(e.missing_required, vec!["gross".to_owned()]);
}

#[test]
fn passport_values_come_from_the_machine_readable_zone_without_a_judgment() {
    let kind = me_core::doc_type("passport").unwrap();
    let mrz = MrzData {
        format: "td3".into(),
        document_code: "P".into(),
        issuing_state: "D".into(),
        surname: "MUSTERMANN".into(),
        given_names: "MAX".into(),
        document_number: "C01X00T47".into(),
        nationality: "D".into(),
        birth_date: Some("1988-03-14".into()),
        sex: "M".into(),
        expiry_date: Some("2031-04-30".into()),
        check_digits_valid: true,
    };
    let cands = vec![cand(
        "m0",
        CandidateKind::Mrz,
        "P<D<<MUSTERMANN<<MAX",
        CandidateValue::Mrz(mrz),
    )];
    let mut d = Scripted::default();
    let e = extract(
        Document {
            title: "pass.jpg",
            segments: &segments(),
        },
        kind,
        &payslip_classification(),
        &cands,
        0.8,
        &mut d,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(d.requests.is_empty(), "nothing left to judge");
    let number = e.graph.values.iter().find(|v| v.slot == "number").unwrap();
    assert_eq!(number.source, ConfidenceSource::Checksum);
    assert!(
        matches!(&number.content, SlotContent::Candidate(c) if c.value == CandidateValue::Identifier("C01X00T47".into()))
    );
    assert!(e.missing_required.is_empty());
}

#[test]
fn gap_fill_values_are_verified_before_use() {
    let kind = me_core::doc_type("payslip").unwrap();
    let base = || Extraction {
        graph: DocumentGraph {
            doc_type: "payslip".into(),
            subject: None,
            subject_confidence: 0.,
            values: vec![],
            models: vec![],
            correction: false,
        },
        missing_required: vec!["gross".into()],
        usage: StageUsage::default(),
    };
    let proposed = vec![cand(
        "g0",
        CandidateKind::Money,
        "4.200,00 €",
        CandidateValue::Money {
            amount: "4200.00".into(),
            currency: "EUR".into(),
        },
    )];
    let mut accepted = base();
    let mut d = Scripted::default()
        .choice("slot_gross", "g0", 0.9)
        .noul("gross::hallucinated", 0.05);
    verify_gap_fill(
        Document {
            title: "p.pdf",
            segments: &segments(),
        },
        kind,
        &proposed,
        &mut accepted,
        0.8,
        &mut d,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(d.requests.len(), 2);
    assert!(
        d.requests[1]
            .1
            .as_object()
            .unwrap()
            .keys()
            .all(|k| k.starts_with("gross::"))
    );
    assert_eq!(
        accepted.graph.values[0].source,
        ConfidenceSource::OpenaiGrounded
    );
    assert!(accepted.missing_required.is_empty());

    let mut rejected = base();
    let mut d = Scripted::default()
        .choice("slot_gross", "g0", 0.9)
        .noul("gross::off_target", 0.85);
    verify_gap_fill(
        Document {
            title: "p.pdf",
            segments: &segments(),
        },
        kind,
        &proposed,
        &mut rejected,
        0.8,
        &mut d,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(rejected.graph.values.is_empty());
    assert_eq!(rejected.missing_required, vec!["gross".to_owned()]);
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
    let many: Vec<Candidate> = (0..300)
        .map(|i| {
            cand(
                &format!("c{i}"),
                CandidateKind::Date,
                &format!("{:02}.01.2020", i % 28 + 1),
                CandidateValue::Date("2020-01-01".into()),
            )
        })
        .collect();
    let (questions, offered) =
        q::extract_questions(me_core::doc_type("payslip").unwrap(), &many, &[]);
    assert!(
        offered
            .iter()
            .all(|(_, ids)| ids.len() <= q::MAX_SLOT_OPTIONS)
    );
    assert!(
        questions
            .as_object()
            .unwrap()
            .values()
            .all(|q| q["criteria"].as_object().is_none_or(|o| o.len() <= 255))
    );
    assert_eq!(q::MODEL, crate::typesafe::MODEL);
}

#[test]
#[ignore = "Two live TypeSafe requests on synthetic text; needs private API configuration"]
fn live_synthetic_payslip_is_classified_and_its_values_selected() {
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
    let kind = c.eager_type().unwrap();
    let e = extract(
        doc,
        kind,
        &c,
        &payslip_candidates(),
        0.8,
        &mut typesafe,
        &cancel,
    )
    .unwrap();
    let pick = |slot: &str| {
        e.graph
            .values
            .iter()
            .find(|v| v.slot == slot)
            .map(|v| match &v.content {
                SlotContent::Candidate(c) => c.id.clone(),
                SlotContent::Category { key } => key.clone(),
            })
    };
    assert_eq!(pick("employer").as_deref(), Some("c0"), "{e:?}");
    assert_eq!(pick("gross").as_deref(), Some("c2"), "{e:?}");
    assert_eq!(pick("net").as_deref(), Some("c3"), "{e:?}");
    eprintln!(
        "{:#?}",
        e.graph
            .values
            .iter()
            .map(|v| (&v.slot, v.confidence, v.check))
            .collect::<Vec<_>>()
    );
}
