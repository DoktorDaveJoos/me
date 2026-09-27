//! Resolve-stage behavior on synthetic documents only.
use super::*;
use crate::{Candidate, CandidateKind, CandidateValue, ResolutionStatus};
use rusqlite::params;

const PASSWORD: &str = "synthetic-graph-password";

fn setup() -> (tempfile::TempDir, Vault) {
    let t = tempfile::tempdir().unwrap();
    let v = Vault::create(&t.path().join("vault"), PASSWORD).unwrap();
    (t, v)
}
fn document(v: &mut Vault, title: &str) -> String {
    let source = id();
    v.db.execute("INSERT INTO source(id,kind,title,received_at,content_fingerprint,sensitivity,retention) VALUES(?,'file',?,strftime('%Y-%m-%dT%H:%M:%fZ','now'),?,'personal','keep')",params![source,title,hash(source.as_bytes())]).unwrap();
    source
}
fn cand(kind: CandidateKind, text: &str, value: CandidateValue) -> Candidate {
    Candidate {
        id: format!("c-{text}"),
        kind,
        text: text.into(),
        value,
        segment_id: "segment".into(),
        start: 0,
        end: text.len(),
        line: text.into(),
        label: None,
        checksum: false,
    }
}
fn value(slot: &str, c: Candidate, confidence: f64) -> SlotValue {
    SlotValue {
        slot: slot.into(),
        content: SlotContent::Candidate(Box::new(c)),
        period: None,
        confidence,
        source: ConfidenceSource::Typesafe,
        value_checked: false,
        check: false,
    }
}
fn date(slot: &str, d: &str) -> SlotValue {
    value(
        slot,
        cand(CandidateKind::Date, d, CandidateValue::Date(d.into())),
        0.95,
    )
}
fn text(slot: &str, s: &str, kind: CandidateKind) -> SlotValue {
    value(slot, cand(kind, s, CandidateValue::Text(s.into())), 0.95)
}
fn money(slot: &str, amount: &str) -> SlotValue {
    value(
        slot,
        cand(
            CandidateKind::Money,
            amount,
            CandidateValue::Money {
                amount: amount.into(),
                currency: "EUR".into(),
            },
        ),
        0.95,
    )
}
fn graph(doc_type: &str, subject: Option<String>, values: Vec<SlotValue>) -> DocumentGraph {
    DocumentGraph {
        doc_type: doc_type.into(),
        subject,
        subject_confidence: 0.95,
        values,
        models: vec!["jev-test".into()],
        correction: false,
    }
}
fn passport(subject: &str, number: &str) -> DocumentGraph {
    let mrz = |value| Candidate {
        checksum: true,
        ..cand(CandidateKind::Mrz, "P<UTOMUSTERMANN<<MAX<<<<", value)
    };
    let slot = |slot: &str, v| SlotValue {
        source: ConfidenceSource::Checksum,
        value_checked: true,
        confidence: 1.,
        ..value(slot, mrz(v), 1.)
    };
    graph(
        "passport",
        Some(subject.into()),
        vec![
            slot("number", CandidateValue::Identifier(number.into())),
            slot("expires", CandidateValue::Date("2031-04-30".into())),
            slot("birth_date", CandidateValue::Date("1988-03-14".into())),
        ],
    )
}
fn count(v: &Vault, sql: &str) -> i64 {
    v.db.query_row(sql, [], |r| r.get(0)).unwrap()
}
fn resolved(v: &Vault, entity: &str, property: &str, as_of: &str) -> Vec<String> {
    let r = v.resolve_fact_at(entity, property, as_of).unwrap();
    r.facts.iter().map(|f| f.value.to_string()).collect()
}
fn linked_label(v: &Vault, entity: &str, property: &str, as_of: &str) -> Option<String> {
    let r = v.resolve_fact_at(entity, property, as_of).unwrap();
    let id = r.facts.first()?.value.as_str()?.to_owned();
    v.db.query_row("SELECT label FROM entity WHERE id=?", [id], |r| r.get(0))
        .ok()
}

#[test]
fn identity_setup_records_anchors_and_household() {
    let (_t, mut v) = setup();
    v.set_identity(
        "Max Mustermann",
        &["Max Beispiel".into()],
        Some("1988-03-14"),
    )
    .unwrap();
    let lena = v
        .add_household_member("Lena Mustermann", "partner")
        .unwrap();
    assert_eq!(
        v.add_household_member("lena mustermann", "partner")
            .unwrap(),
        lena
    );
    v.complete_identity_setup().unwrap();
    let a = v.identity_anchors().unwrap();
    assert_eq!(a.self_name, "Max Mustermann");
    assert_eq!(a.aliases, vec!["Max Beispiel".to_owned()]);
    assert_eq!(a.birth_date.as_deref(), Some("1988-03-14"));
    assert_eq!(a.household.len(), 1);
    assert!(a.setup_complete);
    assert_eq!(a.names().len(), 3);
    assert!(v.add_household_member("Paul", "pet").is_err());
}

#[test]
fn passport_creates_identity_document_links_and_is_idempotent() {
    let (_t, mut v) = setup();
    let me = v.profile_entity_id().unwrap();
    let doc = document(&mut v, "passport.pdf");
    let outcome = v
        .apply_document_graph(&doc, &passport(&me, "C01X00T47"))
        .unwrap();
    assert_eq!(outcome.entities_created, 1);
    let passport_id = linked_label(&v, &me, "person.government_id", "2026-01-01").unwrap();
    assert_eq!(passport_id, "Passport");
    assert_eq!(
        resolved(&v, &me, "person.birth_date", "2026-01-01"),
        vec!["\"1988-03-14\""]
    );
    let before = count(&v, "SELECT count(*) FROM assertion");
    // Re-running the same document, or a second scan of the same passport, adds nothing.
    v.apply_document_graph(&doc, &passport(&me, "C01X00T47"))
        .unwrap();
    let copy = document(&mut v, "passport-copy.jpg");
    let outcome = v
        .apply_document_graph(&copy, &passport(&me, "C01X 00T47"))
        .unwrap();
    assert_eq!(outcome.entities_created, 0);
    assert_eq!(count(&v, "SELECT count(*) FROM assertion"), before);
    assert_eq!(
        count(&v, "SELECT count(*) FROM entity WHERE kind='government_id'"),
        1
    );
    // Checksum values are automatic but not flagged; every decision is a policy decision.
    assert!(v.quick_checks().unwrap().is_empty());
    let a: String =
        v.db.query_row(
            "SELECT assertion_id FROM assertion_review WHERE confidence_source='checksum' LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(v.review_state(&a).unwrap(), ReviewState::Unreviewed);
    assert_eq!(
        count(
            &v,
            "SELECT count(*) FROM decision WHERE actor='policy' AND policy_version IS NULL"
        ),
        0
    );
    // Search lists the identity values.
    assert!(
        count(
            &v,
            "SELECT count(*) FROM collection_item WHERE kind='note' AND title LIKE 'Passport ·%'"
        ) >= 2
    );
    let map = v.constellation(30).unwrap();
    assert_eq!(map.nodes[0].id, me);
    assert!(map.nodes.iter().any(|n| n.kind == "government_id"));
    assert_eq!(map.edges.len(), 1);
    assert!(map.latest.is_some());
}

#[test]
fn payslips_share_one_employer_and_keep_monthly_income_apart() {
    let (_t, mut v) = setup();
    let me = v.profile_entity_id().unwrap();
    for (month, end, employer, gross) in [
        ("2026-01-01", "2026-01-31", "Acme GmbH", "4200.00"),
        ("2026-02-01", "2026-02-28", "ACME", "4300.00"),
    ] {
        let doc = document(&mut v, "payslip.pdf");
        v.apply_document_graph(
            &doc,
            &graph(
                "payslip",
                Some(me.clone()),
                vec![
                    text("employer", employer, CandidateKind::Organization),
                    money("gross", gross),
                    date("period_start", month),
                    date("period_end", end),
                ],
            ),
        )
        .unwrap();
    }
    assert_eq!(
        count(&v, "SELECT count(*) FROM entity WHERE kind='organization'"),
        1
    );
    assert_eq!(
        resolved(&v, &me, "person.gross_income", "2026-01-15"),
        vec![r#"{"amount":"4200","currency":"EUR","period":"month"}"#]
    );
    assert_eq!(
        resolved(&v, &me, "person.gross_income", "2026-02-15"),
        vec![r#"{"amount":"4300","currency":"EUR","period":"month"}"#]
    );
    assert!(resolved(&v, &me, "person.gross_income", "2026-03-15").is_empty());
    assert_eq!(
        linked_label(&v, &me, "person.employer", "2026-02-10").as_deref(),
        Some("Acme GmbH")
    );
    assert!(v.quick_checks().unwrap().is_empty());
}

fn move_in(v: &mut Vault, me: &str, street: &str, postal: &str, date_: &str) {
    let doc = document(v, "meldebescheinigung.pdf");
    v.apply_document_graph(
        &doc,
        &graph(
            "residence_registration",
            Some(me.into()),
            vec![
                text("street", street, CandidateKind::Street),
                value(
                    "postal_code",
                    cand(
                        CandidateKind::PostalCode,
                        postal,
                        CandidateValue::Identifier(postal.into()),
                    ),
                    0.95,
                ),
                date("move_in", date_),
            ],
        ),
    )
    .unwrap();
}
fn residence_history(forward: bool) -> Vec<Option<String>> {
    let (_t, mut v) = setup();
    let me = v.profile_entity_id().unwrap();
    let moves = [
        ("Musterstraße 1", "10115", "2019-04-01"),
        ("Beispielweg 7", "80331", "2023-05-01"),
    ];
    let order: Vec<_> = if forward {
        moves.to_vec()
    } else {
        moves.iter().rev().cloned().collect()
    };
    for (street, postal, day) in order {
        move_in(&mut v, &me, street, postal, day);
    }
    assert_eq!(v.quick_checks().unwrap().len(), 0, "no false conflict");
    ["2018-01-01", "2020-06-01", "2024-06-01"]
        .iter()
        .map(|d| linked_label(&v, &me, "person.residence", d))
        .collect()
}

#[test]
fn newer_documents_supersede_older_ones_in_any_import_order() {
    let expected = vec![
        None,
        Some("Musterstraße 1".to_owned()),
        Some("Beispielweg 7".to_owned()),
    ];
    assert_eq!(residence_history(true), expected);
    assert_eq!(residence_history(false), expected);
}

#[test]
fn same_street_spelled_differently_is_one_address() {
    let (_t, mut v) = setup();
    let me = v.profile_entity_id().unwrap();
    move_in(&mut v, &me, "Musterstraße 1", "10115", "2019-04-01");
    move_in(&mut v, &me, "Musterstr. 1", "10115", "2019-04-01");
    assert_eq!(
        count(&v, "SELECT count(*) FROM entity WHERE kind='address'"),
        1
    );
}

#[test]
fn conflicting_identifiers_and_low_confidence_become_quick_checks() {
    let (_t, mut v) = setup();
    let me = v.profile_entity_id().unwrap();
    let tax = |id: &str| {
        value(
            "tax_id",
            cand(
                CandidateKind::TaxId,
                id,
                CandidateValue::Identifier(id.into()),
            ),
            0.95,
        )
    };
    for id in ["12345678903", "98765432106"] {
        let doc = document(&mut v, "steuerbescheid.pdf");
        v.apply_document_graph(
            &doc,
            &graph("tax_assessment", Some(me.clone()), vec![tax(id)]),
        )
        .unwrap();
    }
    assert_eq!(
        v.resolve_fact_at(&me, "person.tax_id", "2026-01-01")
            .unwrap()
            .status,
        ResolutionStatus::Conflict
    );
    let checks = v.quick_checks().unwrap();
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].items.len(), 2);
    assert!(checks[0].items.iter().all(|i| i.reason == "conflict"));
    // Rejecting one value resolves the conflict; confirming the other reviews it.
    let (wrong, right) = (
        checks[0].items[1].assertion.clone(),
        checks[0].items[0].assertion.clone(),
    );
    v.reject_assertion(&wrong).unwrap();
    v.confirm_assertion(&right).unwrap();
    assert_eq!(v.review_state(&right).unwrap(), ReviewState::Reviewed);
    assert!(v.quick_checks().unwrap().is_empty());
    assert_eq!(
        v.resolve_fact_at(&me, "person.tax_id", "2026-01-01")
            .unwrap()
            .status,
        ResolutionStatus::Resolved
    );

    let doc = document(&mut v, "bank.pdf");
    let mut iban = value(
        "iban",
        cand(
            CandidateKind::Iban,
            "DE89370400440532013000",
            CandidateValue::Identifier("DE89370400440532013000".into()),
        ),
        0.65,
    );
    iban.value_checked = true;
    v.apply_document_graph(
        &doc,
        &graph("bank_account_document", Some(me.clone()), vec![iban]),
    )
    .unwrap();
    let checks = v.quick_checks().unwrap();
    assert!(
        checks
            .iter()
            .flat_map(|g| &g.items)
            .any(|i| i.reason == "low_confidence" && i.label == "IBAN")
    );
}

#[test]
fn user_decisions_are_never_overridden_by_later_imports() {
    let (_t, mut v) = setup();
    let me = v.profile_entity_id().unwrap();
    let doc = document(&mut v, "passport.pdf");
    v.apply_document_graph(&doc, &passport(&me, "C01X00T47"))
        .unwrap();
    let birth: String =
        v.db.query_row(
            "SELECT id FROM assertion WHERE property_key='person.birth_date'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    v.reject_assertion(&birth).unwrap();
    let again = document(&mut v, "passport-again.pdf");
    v.apply_document_graph(&again, &passport(&me, "C01X00T47"))
        .unwrap();
    assert_eq!(v.review_state(&birth).unwrap(), ReviewState::Reviewed);
    assert!(resolved(&v, &me, "person.birth_date", "2026-01-01").is_empty());
}

#[test]
fn documents_about_unknown_people_propose_a_household_member_after_three() {
    let (_t, mut v) = setup();
    for i in 0..3 {
        let doc = document(&mut v, &format!("letter-{i}.pdf"));
        v.save_document_profile(
            &doc,
            "insurance",
            0.9,
            Some("insurance_policy"),
            Some(0.9),
            None,
            Some("Lena Mustermann"),
            Some("jev-test"),
        )
        .unwrap();
        assert_eq!(v.household_proposals().unwrap().len(), usize::from(i == 2));
    }
    let proposal = &v.household_proposals().unwrap()[0];
    assert_eq!(
        (proposal.name.as_str(), proposal.documents),
        ("Lena Mustermann", 3)
    );
    v.add_household_member("Lena Mustermann", "partner")
        .unwrap();
    assert!(v.household_proposals().unwrap().is_empty());
    assert_eq!(
        v.family_counts().unwrap(),
        vec![("insurance".to_owned(), 3)]
    );
    let other = document(&mut v, "x");
    assert!(
        v.save_document_profile(
            &other,
            "insurance",
            0.9,
            Some("payslip"),
            None,
            None,
            None,
            None
        )
        .is_err()
    );
}

#[test]
fn subjectless_documents_keep_entity_facts_but_no_personal_facts() {
    let (_t, mut v) = setup();
    let doc = document(&mut v, "policy.pdf");
    v.apply_document_graph(
        &doc,
        &graph(
            "insurance_policy",
            None,
            vec![
                text("insurer", "HUK-COBURG", CandidateKind::Organization),
                value(
                    "number",
                    cand(
                        CandidateKind::Identifier,
                        "KV-123/456",
                        CandidateValue::Identifier("KV-123/456".into()),
                    ),
                    0.9,
                ),
            ],
        ),
    )
    .unwrap();
    assert_eq!(
        count(
            &v,
            "SELECT count(*) FROM entity WHERE kind='insurance_contract'"
        ),
        1
    );
    let me = v.profile_entity_id().unwrap();
    assert_eq!(
        count(
            &v,
            &format!(
                "SELECT count(*) FROM assertion WHERE subject_id='{me}' OR object_entity_id='{me}'"
            )
        ),
        0
    );
}

#[test]
fn stage_cache_and_equivalence_and_merge_proposals() {
    let (_t, mut v) = setup();
    v.save_stage_output(
        "hash",
        "classify",
        "v1",
        &json!({"family":"tax"}),
        Some("jev-1.13.0"),
        Some(12),
    )
    .unwrap();
    assert_eq!(
        v.stage_output("hash", "classify", "v1").unwrap(),
        Some(json!({"family":"tax"}))
    );
    assert_eq!(v.stage_output("hash", "classify", "v2").unwrap(), None);

    let me = v.profile_entity_id().unwrap();
    for name in ["Techniker Krankenkasse", "Techniker KK"] {
        let doc = document(&mut v, "tk.pdf");
        v.apply_document_graph(
            &doc,
            &graph(
                "health_insurance_notice",
                Some(me.clone()),
                vec![text("insurer", name, CandidateKind::Organization)],
            ),
        )
        .unwrap();
    }
    let candidates = v.merge_candidates(10).unwrap();
    assert_eq!(candidates.len(), 1);
    v.record_merge_judgment(&candidates[0].a, &candidates[0].b, 0.9, true)
        .unwrap();
    assert!(v.merge_candidates(10).unwrap().is_empty());
    assert_eq!(v.merge_proposals().unwrap().len(), 1);
    v.decide_merge_proposal(&candidates[0].a, &candidates[0].b, true)
        .unwrap();
    assert!(v.merge_proposals().unwrap().is_empty());

    let occupation = |s: &str| text("occupation", s, CandidateKind::LabelValue);
    for job in ["Software-Entwickler", "Softwareentwickler"] {
        let doc = document(&mut v, "vertrag.pdf");
        v.apply_document_graph(
            &doc,
            &graph(
                "employment_contract",
                Some(me.clone()),
                vec![
                    text("employer", "Acme GmbH", CandidateKind::Organization),
                    occupation(job),
                    date("start", "2020-01-01"),
                ],
            ),
        )
        .unwrap();
    }
    let conflicts = v.open_value_conflicts(10).unwrap();
    assert_eq!(conflicts.len(), 1);
    v.resolve_equivalent_values(&conflicts[0].first, &conflicts[0].second)
        .unwrap();
    assert!(v.open_value_conflicts(10).unwrap().is_empty());
    assert_eq!(
        resolved(&v, &me, "person.occupation", "2021-01-01").len(),
        1
    );
}

#[test]
fn review_outcomes_tune_the_check_threshold() {
    let (_t, mut v) = setup();
    assert_eq!(v.check_threshold().unwrap(), DEFAULT_CHECK_THRESHOLD);
    let me = v.profile_entity_id().unwrap();
    let mut ids = Vec::new();
    for i in 0..20 {
        let doc = document(&mut v, "p.pdf");
        v.apply_document_graph(
            &doc,
            &graph(
                "health_insurance_notice",
                Some(me.clone()),
                vec![text(
                    "insurer",
                    &format!("Kasse {i}"),
                    CandidateKind::Organization,
                )],
            ),
        )
        .unwrap();
        ids.push(
            v.db.query_row(
                "SELECT assertion_id FROM assertion_review ORDER BY rowid DESC LIMIT 1",
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap(),
        );
    }
    for id in &ids {
        v.confirm_assertion(id).unwrap();
    }
    assert_eq!(v.check_threshold().unwrap(), 0.7);
}

#[test]
fn next_day_handles_month_and_leap_year_ends() {
    assert_eq!(next_day("2026-01-31").as_deref(), Some("2026-02-01"));
    assert_eq!(next_day("2024-02-28").as_deref(), Some("2024-02-29"));
    assert_eq!(next_day("2026-12-31").as_deref(), Some("2027-01-01"));
    assert_eq!(
        normalize_organization("HUK-COBURG Versicherung AG"),
        "huk coburg versicherung"
    );
    assert_eq!(normalize_organization("Acme GmbH & Co. KG"), "acme");
    assert_eq!(normalize_name("Jürgen  Groß"), "juergen gross");
}

#[test]
#[ignore = "Scale measurement; run with --release --ignored --nocapture"]
fn thousand_documents_resolve_and_read_back_quickly() {
    let (_t, mut v) = setup();
    let me = v.profile_entity_id().unwrap();
    let started = std::time::Instant::now();
    for i in 0..1000u32 {
        let month = i % 12 + 1;
        let year = 2000 + i / 12;
        let doc = document(&mut v, "payslip.pdf");
        v.apply_document_graph(
            &doc,
            &graph(
                "payslip",
                Some(me.clone()),
                vec![
                    text(
                        "employer",
                        &format!("Employer {}", i % 12),
                        CandidateKind::Organization,
                    ),
                    money("gross", &format!("{}.00", 3000 + i)),
                    date("period_start", &format!("{year}-{month:02}-01")),
                    date("period_end", &format!("{year}-{month:02}-28")),
                ],
            ),
        )
        .unwrap();
    }
    let resolved = started.elapsed();
    let started = std::time::Instant::now();
    let map = v.constellation(19).unwrap();
    let checks = v.quick_checks().unwrap();
    let read = started.elapsed();
    eprintln!(
        "1000 documents resolved in {resolved:?}; constellation ({} nodes) and {} check groups read in {read:?}",
        map.nodes.len(),
        checks.len()
    );
    assert_eq!(map.nodes.len(), 13);
}
