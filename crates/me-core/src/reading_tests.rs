//! Read-store behavior on synthetic documents only.
use super::*;
use crate::{
    Candidate, CandidateKind, CandidateValue, ConfidenceSource, DocumentGraph, SlotContent,
    SlotValue, vault::hash,
};
use std::{
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
};

const SYNTHETIC_PASSWORD: &str = "synthetic-reading-password";

/// A vault whose temporary folder lives exactly as long as the vault.
struct TempVault {
    vault: Vault,
    _dir: tempfile::TempDir,
}
impl Deref for TempVault {
    type Target = Vault;
    fn deref(&self) -> &Vault {
        &self.vault
    }
}
impl DerefMut for TempVault {
    fn deref_mut(&mut self) -> &mut Vault {
        &mut self.vault
    }
}
/// A vault folder that outlives the vault, for close-and-unlock fixtures.
struct VaultRoot {
    path: PathBuf,
    _dir: tempfile::TempDir,
}
impl Deref for VaultRoot {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.path
    }
}

/// A personal, kept document source with its collection item and evaluation row.
fn add_synthetic_source(vault: &mut Vault, title: &str) -> String {
    let source = id();
    vault.db.execute("INSERT INTO source(id,kind,title,received_at,content_fingerprint,sensitivity,retention) VALUES(?,'file',?,strftime('%Y-%m-%dT%H:%M:%fZ','now'),?,'personal','keep')", params![source, title, hash(source.as_bytes())]).unwrap();
    vault
        .db
        .execute(
            "INSERT INTO collection_item(stable_id,title,kind,source_id,extension) VALUES(?,?,'document',?,'txt')",
            params![id(), title, source],
        )
        .unwrap();
    vault
        .db
        .execute(
            "INSERT INTO document_evaluation(source_id,state) VALUES(?,'queued')",
            [&source],
        )
        .unwrap();
    source
}
fn synthetic_payslip_source() -> (TempVault, String) {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::create(&dir.path().join("vault"), SYNTHETIC_PASSWORD).unwrap();
    let mut vault = TempVault { vault, _dir: dir };
    let source = add_synthetic_source(&mut vault, "payslip.txt");
    (vault, source)
}
fn synthetic_vault_on_disk() -> (Vault, String, VaultRoot) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault");
    let mut vault = Vault::create(&path, SYNTHETIC_PASSWORD).unwrap();
    let source = add_synthetic_source(&mut vault, "payslip.txt");
    (vault, source, VaultRoot { path, _dir: dir })
}
fn candidate_value(kind: CandidateKind, text: &str, value: CandidateValue) -> SlotValue {
    SlotValue {
        slot: String::new(),
        content: SlotContent::Candidate(Box::new(Candidate {
            id: String::new(),
            kind,
            text: text.into(),
            value,
            segment_id: "seg".into(),
            start: 0,
            end: text.len(),
            line: text.into(),
            label: None,
            checksum: false,
        })),
        period: None,
        confidence: 0.95,
        source: ConfidenceSource::Typesafe,
        value_checked: false,
        check: false,
    }
}
fn date_value(d: &str) -> SlotValue {
    candidate_value(CandidateKind::Date, d, CandidateValue::Date(d.into()))
}

fn money(slot: &str, amount: &str, start: usize) -> SlotValue {
    SlotValue {
        slot: slot.into(),
        content: SlotContent::Candidate(Box::new(Candidate {
            id: String::new(),
            kind: CandidateKind::Money,
            text: amount.into(),
            value: CandidateValue::Money {
                amount: amount.into(),
                currency: "EUR".into(),
            },
            segment_id: "seg".into(),
            start,
            end: start + amount.len(),
            line: format!("Betrag {amount}"),
            label: Some("Betrag".into()),
            checksum: false,
        })),
        period: None,
        confidence: 0.95,
        source: ConfidenceSource::Typesafe,
        value_checked: false,
        check: false,
    }
}
fn month(start: &str, end: &str) -> SlotValue {
    SlotValue {
        slot: "pay_month".into(),
        content: SlotContent::Candidate(Box::new(Candidate {
            id: String::new(),
            kind: CandidateKind::Period,
            text: "Januar".into(),
            value: CandidateValue::Period {
                start: start.into(),
                end: end.into(),
            },
            segment_id: "seg".into(),
            start: 0,
            end: 6,
            line: "Januar".into(),
            label: None,
            checksum: false,
        })),
        period: None,
        confidence: 0.95,
        source: ConfidenceSource::Typesafe,
        value_checked: false,
        check: false,
    }
}
fn payslip(subject: &str, values: Vec<SlotValue>) -> DocumentGraph {
    DocumentGraph {
        doc_type: "payslip".into(),
        subject: Some(subject.into()),
        subject_confidence: 0.95,
        values,
        models: vec![],
        correction: false,
    }
}
fn fact(label: &str, value: &str, slot: Option<&str>) -> ReadFact {
    ReadFact {
        label: label.into(),
        value: value.into(),
        segment_id: "seg".into(),
        start: 0,
        end: value.len(),
        context: String::new(),
        owner: "self".into(),
        owner_entity: None,
        owner_name: None,
        period: Some("document".into()),
        slot: slot.map(str::to_owned),
        state: FactState::Verified,
        confidence: Some(0.95),
    }
}
fn employer(name: &str) -> SlotValue {
    SlotValue {
        slot: "employer".into(),
        ..candidate_value(
            CandidateKind::Organization,
            name,
            CandidateValue::Text(name.into()),
        )
    }
}
fn tax_id(number: &str) -> SlotValue {
    SlotValue {
        slot: "tax_id".into(),
        ..candidate_value(
            CandidateKind::TaxId,
            number,
            CandidateValue::Identifier(number.into()),
        )
    }
}
fn january() -> SlotValue {
    month("2026-01-01", "2026-01-31")
}
fn read(run: &str, graph: DocumentGraph) -> DocumentRead {
    DocumentRead {
        run_id: run.into(),
        graph: Some(graph),
        ..Default::default()
    }
}
fn count(vault: &Vault, sql: &str) -> i64 {
    vault.db.query_row(sql, [], |r| r.get(0)).unwrap()
}
fn conflicts(vault: &Vault) -> i64 {
    count(
        vault,
        "SELECT count(*) FROM assertion_review WHERE check_reason='conflict'",
    )
}
/// Stored values of a property that are accepted now.
fn accepted(vault: &Vault, property: &str) -> Vec<String> {
    vault
        .db
        .prepare("SELECT value_json FROM assertion_state WHERE property_key=? AND state='accept' ORDER BY value_json")
        .unwrap()
        .query_map([property], |r| r.get(0))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap()
}
/// Accepted values of a property flagged as conflicting.
fn flagged(vault: &Vault, property: &str) -> i64 {
    vault
        .db
        .query_row(
            "SELECT count(*) FROM assertion_state a JOIN assertion_review r ON r.assertion_id=a.id WHERE a.property_key=? AND a.state='accept' AND r.check_reason='conflict'",
            [property],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn a_read_stores_profile_values_document_facts_and_uninterpreted_values() {
    let (mut vault, source) = synthetic_payslip_source();
    let me = vault.profile_entity_id().unwrap();
    let read = DocumentRead {
        run_id: "run-1".into(),
        graph: Some(payslip(
            &me,
            vec![
                money("wage_tax", "1032.58", 10),
                month("2026-01-01", "2026-01-31"),
            ],
        )),
        facts: vec![
            fact("Lohnsteuer", "1.032,58", Some("wage_tax")),
            fact("Kostenstelle", "4711", None),
        ],
        uninterpreted: vec![crate::Uncovered {
            segment_id: "seg".into(),
            start: 40,
            end: 46,
            kind: CandidateKind::Amount,
            label: Some("KV-Beitrag".into()),
            text: "435,21".into(),
            line: "KV-Beitrag 435,21".into(),
        }],
        rejected: [("value_not_in_quote".to_owned(), 1)].into(),
    };
    let outcome = vault.apply_read(&source, &read).unwrap();
    assert_eq!(
        outcome,
        ReadOutcome {
            values: 2,
            in_profile: 1,
            checks: 0,
            uninterpreted: 1
        }
    );
    let linked: i64 = vault
        .db
        .query_row(
            "SELECT count(*) FROM document_fact WHERE source_id=? AND assertion_id IS NOT NULL",
            [&source],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(linked, 1);
    let (from, to): (String, Option<String>) = vault
        .db
        .query_row(
            "SELECT valid_from,valid_to FROM assertion_state WHERE property_key='person.wage_tax' AND state='accept'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        (from.as_str(), to.as_deref()),
        ("2026-01-01", Some("2026-02-01"))
    );
    // The value nobody interpreted stays visible, and the summary keeps counts and codes.
    assert_eq!(
        count(
            &vault,
            "SELECT count(*) FROM document_fact WHERE state='uninterpreted' AND owner='unknown' AND assertion_id IS NULL"
        ),
        1
    );
    let (values, in_profile, uninterpreted, rejected, policy): (i64, i64, i64, String, String) =
        vault
            .db
            .query_row(
                "SELECT values_read,in_profile,uninterpreted,rejected_json,policy FROM read_summary WHERE source_id=?",
                [&source],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
    assert_eq!((values, in_profile, uninterpreted), (2, 1, 1));
    assert_eq!(rejected, r#"{"value_not_in_quote":1}"#);
    assert_eq!(policy, "import-graph-v2");
}

#[test]
fn the_imports_list_reports_the_counts_of_a_finished_read() {
    let (mut vault, source) = synthetic_payslip_source();
    let unread = add_synthetic_source(&mut vault, "unread.txt");
    let me = vault.profile_entity_id().unwrap();
    let read = DocumentRead {
        run_id: "run-1".into(),
        graph: Some(payslip(
            &me,
            vec![money("wage_tax", "1032.58", 10), january()],
        )),
        facts: vec![
            fact("Lohnsteuer", "1.032,58", Some("wage_tax")),
            fact("Kostenstelle", "4711", None),
        ],
        uninterpreted: vec![crate::Uncovered {
            segment_id: "seg".into(),
            start: 40,
            end: 46,
            kind: CandidateKind::Amount,
            label: Some("KV-Beitrag".into()),
            text: "435,21".into(),
            line: "KV-Beitrag 435,21".into(),
        }],
        rejected: Default::default(),
    };
    vault.apply_read(&source, &read).unwrap();
    let item = |source: &str| -> u64 {
        count(
            &vault,
            &format!("SELECT local_id FROM collection_item WHERE source_id='{source}'"),
        ) as u64
    };
    let jobs = vault.import_jobs().unwrap();
    let job = |item: u64| jobs.iter().find(|j| j.item == item).unwrap();
    assert_eq!(
        job(item(&source)).read,
        Some(crate::ReadCounts {
            values: 2,
            in_profile: 1,
            checks: 0,
            uninterpreted: 1
        })
    );
    // A file without a stored read has no counts, whatever its state says.
    assert_eq!(job(item(&unread)).read, None);
}

#[test]
fn twelve_monthly_payslips_form_a_timeline_without_conflicts() {
    let (mut vault, _) = synthetic_payslip_source();
    let me = vault.profile_entity_id().unwrap();
    for m in 1..=12u32 {
        let source = add_synthetic_source(&mut vault, &format!("payslip-{m}"));
        let last = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][m as usize - 1];
        let amount = format!("{}.00", 1000 + m);
        let read = DocumentRead {
            run_id: format!("run-{m}"),
            graph: Some(payslip(
                &me,
                vec![
                    money("wage_tax", &amount, 10),
                    month(&format!("2026-{m:02}-01"), &format!("2026-{m:02}-{last}")),
                ],
            )),
            ..Default::default()
        };
        vault.apply_read(&source, &read).unwrap();
    }
    // `sum` over only NULL check reasons is NULL, so each row counts 0 or 1 explicitly.
    let (count, conflicts): (i64, i64) = vault
        .db
        .query_row(
            "SELECT count(*),sum(coalesce(r.check_reason,'')='conflict') FROM assertion_state a JOIN assertion_review r ON r.assertion_id=a.id WHERE a.property_key='person.wage_tax' AND a.state='accept'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((count, conflicts), (12, 0));
}

#[test]
fn monthly_and_annual_values_never_conflict() {
    let (mut vault, source) = synthetic_payslip_source();
    let me = vault.profile_entity_id().unwrap();
    let read = DocumentRead {
        run_id: "m".into(),
        graph: Some(payslip(
            &me,
            vec![
                money("wage_tax", "1032.58", 10),
                month("2026-01-01", "2026-01-31"),
            ],
        )),
        ..Default::default()
    };
    vault.apply_read(&source, &read).unwrap();
    let certificate = add_synthetic_source(&mut vault, "lstb");
    let mut annual = payslip(&me, vec![money("wage_tax", "12390.96", 10)]);
    annual.doc_type = "wage_tax_certificate".into();
    annual.values.push(SlotValue {
        slot: "period_start".into(),
        ..date_value("2026-01-01")
    });
    annual.values.push(SlotValue {
        slot: "period_end".into(),
        ..date_value("2026-12-31")
    });
    vault
        .apply_read(
            &certificate,
            &DocumentRead {
                run_id: "y".into(),
                graph: Some(annual),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(conflicts(&vault), 0);
    assert_eq!(
        count(
            &vault,
            "SELECT count(*) FROM assertion_state WHERE property_key='person.wage_tax' AND state='accept'"
        ),
        2
    );
}

#[test]
fn a_tax_year_gives_assessment_amounts_their_year() {
    let (mut vault, source) = synthetic_payslip_source();
    let me = vault.profile_entity_id().unwrap();
    let mut graph = payslip(
        &me,
        vec![
            money("taxable_income", "48210.00", 10),
            SlotValue {
                slot: "tax_year".into(),
                ..month("2025-01-01", "2025-12-31")
            },
        ],
    );
    graph.doc_type = "tax_assessment".into();
    vault
        .apply_read(
            &source,
            &DocumentRead {
                run_id: "t".into(),
                graph: Some(graph),
                ..Default::default()
            },
        )
        .unwrap();
    let (from, to): (String, Option<String>) = vault
        .db
        .query_row(
            "SELECT valid_from,valid_to FROM assertion_state WHERE property_key='person.taxable_income' AND state='accept'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        (from.as_str(), to.as_deref()),
        ("2025-01-01", Some("2026-01-01"))
    );
}

#[test]
fn a_reread_retracts_automatic_values_it_no_longer_finds_but_keeps_user_decisions() {
    let (mut vault, source) = synthetic_payslip_source();
    let me = vault.profile_entity_id().unwrap();
    let first = DocumentRead {
        run_id: "a".into(),
        graph: Some(payslip(
            &me,
            vec![
                money("wage_tax", "1032.58", 10),
                money("church_tax", "10.00", 30),
                month("2026-01-01", "2026-01-31"),
            ],
        )),
        ..Default::default()
    };
    vault.apply_read(&source, &first).unwrap();
    let church: String = vault
        .db
        .query_row(
            "SELECT id FROM assertion_state WHERE property_key='person.church_tax' AND state='accept'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    vault.confirm_assertion(&church).unwrap();
    let second = DocumentRead {
        run_id: "b".into(),
        graph: Some(payslip(&me, vec![month("2026-01-01", "2026-01-31")])),
        ..Default::default()
    };
    vault.apply_read(&source, &second).unwrap();
    let wage: i64 = vault
        .db
        .query_row(
            "SELECT count(*) FROM assertion_state WHERE property_key='person.wage_tax' AND state='accept'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let kept: i64 = vault
        .db
        .query_row(
            "SELECT count(*) FROM assertion_state WHERE property_key='person.church_tax' AND state='accept'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!((wage, kept), (0, 1));
    assert_eq!(
        count(
            &vault,
            "SELECT count(*) FROM decision WHERE actor='policy' AND action='retract' AND reason_code='not_found_on_reread' AND policy_version='import-graph-v2'"
        ),
        1
    );
}

#[test]
fn a_reread_replaces_document_facts_and_links_values_the_user_confirmed() {
    let (mut vault, source) = synthetic_payslip_source();
    let me = vault.profile_entity_id().unwrap();
    let read = DocumentRead {
        run_id: "a".into(),
        graph: Some(payslip(
            &me,
            vec![
                money("wage_tax", "1032.58", 10),
                month("2026-01-01", "2026-01-31"),
            ],
        )),
        facts: vec![
            fact("Lohnsteuer", "1.032,58", Some("wage_tax")),
            fact("Kostenstelle", "4711", None),
        ],
        ..Default::default()
    };
    vault.apply_read(&source, &read).unwrap();
    let wage: String = vault
        .db
        .query_row(
            "SELECT id FROM assertion_state WHERE property_key='person.wage_tax' AND state='accept'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    vault.confirm_assertion(&wage).unwrap();
    let again = DocumentRead {
        run_id: "b".into(),
        ..read
    };
    let outcome = vault.apply_read(&source, &again).unwrap();
    assert_eq!((outcome.values, outcome.in_profile), (2, 1));
    let (rows, linked): (i64, Option<String>) = vault
        .db
        .query_row(
            "SELECT count(*),max(assertion_id) FROM document_fact WHERE source_id=? AND run_id='b'",
            [&source],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((rows, linked.as_deref()), (2, Some(wage.as_str())));
    assert_eq!(count(&vault, "SELECT count(*) FROM document_fact"), 2);
    assert_eq!(
        count(&vault, "SELECT count(*) FROM read_summary WHERE run_id='b'"),
        1
    );
    assert_eq!(
        vault.review_state(&wage).unwrap(),
        crate::ReviewState::Reviewed
    );
}

#[test]
fn a_reread_of_a_superseded_document_keeps_its_closed_value() {
    let (mut vault, first) = synthetic_payslip_source();
    let me = vault.profile_entity_id().unwrap();
    let registration = |run: &str, street: &str, postal: &str, day: &str| DocumentRead {
        run_id: run.into(),
        graph: Some(DocumentGraph {
            doc_type: "residence_registration".into(),
            subject: Some(me.clone()),
            subject_confidence: 0.95,
            values: vec![
                SlotValue {
                    slot: "street".into(),
                    ..candidate_value(
                        CandidateKind::Street,
                        street,
                        CandidateValue::Text(street.into()),
                    )
                },
                SlotValue {
                    slot: "postal_code".into(),
                    ..candidate_value(
                        CandidateKind::PostalCode,
                        postal,
                        CandidateValue::Identifier(postal.into()),
                    )
                },
                SlotValue {
                    slot: "move_in".into(),
                    ..date_value(day)
                },
            ],
            models: vec![],
            correction: false,
        }),
        ..Default::default()
    };
    let old = registration("a", "Musterstraße 1", "10115", "2019-04-01");
    vault.apply_read(&first, &old).unwrap();
    let second = add_synthetic_source(&mut vault, "meldebescheinigung-neu");
    vault
        .apply_read(
            &second,
            &registration("b", "Beispielweg 7", "80331", "2023-05-01"),
        )
        .unwrap();
    // Reading the older registration again still finds its value; the closed
    // replacement that carries it must not be retracted as missing.
    vault
        .apply_read(
            &first,
            &DocumentRead {
                run_id: "c".into(),
                ..old
            },
        )
        .unwrap();
    let at = |day: &str| {
        vault
            .resolve_fact_at(&me, "person.residence", day)
            .unwrap()
            .facts
            .len()
    };
    assert_eq!((at("2020-06-01"), at("2024-06-01")), (1, 1));
    assert_eq!(conflicts(&vault), 0);
}

#[test]
fn facts_of_a_superseded_value_link_the_closed_replacement() {
    let (mut vault, first) = synthetic_payslip_source();
    let me = vault.profile_entity_id().unwrap();
    let notice = |run: &str, number: &str, day: &str| DocumentRead {
        run_id: run.into(),
        graph: Some(DocumentGraph {
            doc_type: "tax_assessment".into(),
            subject: Some(me.clone()),
            subject_confidence: 0.95,
            values: vec![
                SlotValue {
                    slot: "tax_number".into(),
                    ..candidate_value(
                        CandidateKind::Identifier,
                        number,
                        CandidateValue::Identifier(number.into()),
                    )
                },
                SlotValue {
                    slot: "document_date".into(),
                    ..date_value(day)
                },
            ],
            models: vec![],
            correction: false,
        }),
        facts: vec![fact("Steuernummer", number, Some("tax_number"))],
        ..Default::default()
    };
    let old = notice("a", "21/815/08150", "2020-03-01");
    vault.apply_read(&first, &old).unwrap();
    let second = add_synthetic_source(&mut vault, "bescheid-neu");
    vault
        .apply_read(&second, &notice("b", "21/815/99999", "2023-03-01"))
        .unwrap();
    let outcome = vault
        .apply_read(
            &first,
            &DocumentRead {
                run_id: "c".into(),
                ..old
            },
        )
        .unwrap();
    assert_eq!(outcome.in_profile, 1);
    let (state, to): (String, Option<String>) = vault
        .db
        .query_row(
            "SELECT a.state,a.valid_to FROM document_fact f JOIN assertion_state a ON a.id=f.assertion_id WHERE f.source_id=?",
            [&first],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        (state.as_str(), to.as_deref()),
        ("accept", Some("2023-03-01"))
    );
}

#[test]
fn a_correction_supersedes_the_earlier_value_for_the_same_period() {
    let (mut vault, source) = synthetic_payslip_source();
    let me = vault.profile_entity_id().unwrap();
    let original = DocumentRead {
        run_id: "o".into(),
        graph: Some(payslip(
            &me,
            vec![
                employer("Acme GmbH"),
                money("wage_tax", "1032.58", 10),
                month("2026-01-01", "2026-01-31"),
            ],
        )),
        ..Default::default()
    };
    vault.apply_read(&source, &original).unwrap();
    let fixed = add_synthetic_source(&mut vault, "correction");
    let mut graph = payslip(
        &me,
        vec![
            employer("Acme GmbH"),
            money("wage_tax", "1040.00", 10),
            month("2026-01-01", "2026-01-31"),
        ],
    );
    graph.correction = true;
    vault
        .apply_read(
            &fixed,
            &DocumentRead {
                run_id: "c".into(),
                graph: Some(graph),
                ..Default::default()
            },
        )
        .unwrap();
    let accepted = |vault: &Vault| -> Vec<String> {
        vault
            .db
            .prepare("SELECT value_json FROM assertion_state WHERE property_key='person.wage_tax' AND state='accept'")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap()
    };
    // Amounts are stored as exact normalized decimals: 1040.00 becomes "1040".
    let values = accepted(&vault);
    assert_eq!(values.len(), 1);
    assert!(values[0].contains(r#""amount":"1040""#));
    assert_eq!(conflicts(&vault), 0);
    // Reading the original again does not bring the corrected value back.
    vault
        .apply_read(
            &source,
            &DocumentRead {
                run_id: "o2".into(),
                ..original
            },
        )
        .unwrap();
    let values = accepted(&vault);
    assert_eq!(values.len(), 1);
    assert!(values[0].contains(r#""amount":"1040""#));
    assert_eq!(conflicts(&vault), 0);
}

#[test]
fn only_personal_kept_documents_are_read() {
    let (mut vault, source) = synthetic_payslip_source();
    vault
        .db
        .execute(
            "UPDATE source SET sensitivity='restricted' WHERE id=?",
            [&source],
        )
        .unwrap();
    let read = DocumentRead {
        run_id: "r".into(),
        facts: vec![fact("Kostenstelle", "4711", None)],
        ..Default::default()
    };
    assert!(vault.apply_read(&source, &read).is_err());
    assert_eq!(count(&vault, "SELECT count(*) FROM document_fact"), 0);
}

#[test]
fn completing_a_read_run_finishes_its_run_and_job() {
    let (mut vault, source) = synthetic_payslip_source();
    vault.db.execute("INSERT INTO extraction_run(id,source_id,provider,model,pipeline_version,schema_version,started_at,status) VALUES('run-1',?,'typesafe','synthetic','read-v1',1,strftime('%Y-%m-%dT%H:%M:%fZ','now'),'running')", [&source]).unwrap();
    vault.db.execute("INSERT INTO job(id,source_id,kind,dedup_key,state,last_error_code) VALUES('job-1',?,'extract_facts','extract:synthetic','running','provider_busy')", [&source]).unwrap();
    vault.complete_read_run("run-1").unwrap();
    let run: String = vault
        .db
        .query_row(
            "SELECT status FROM extraction_run WHERE id='run-1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let (job, error): (String, Option<String>) = vault
        .db
        .query_row(
            "SELECT state,last_error_code FROM job WHERE id='job-1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        (run.as_str(), job.as_str(), error),
        ("succeeded", "done", None)
    );
}

#[test]
fn migration_sixteen_requeues_documents_read_by_the_tiered_reader() {
    let (mut vault, source, root) = synthetic_vault_on_disk();
    // Read deeply already (a finished read job), and never classified: both stay done.
    let deep = add_synthetic_source(&mut vault, "payslip-deep.txt");
    let unclassified = add_synthetic_source(&mut vault, "notes.txt");
    // A version 15 vault: the legacy-fixture helper is not used because it removes
    // the version 15 tables (document_profile, priority) that this migration reads.
    vault.db.execute_batch(&format!(
        "INSERT INTO document_profile(source_id,family,doc_type,family_confidence,type_confidence,tier,graph_state,classified_at) VALUES('{source}','employment','payslip',0.9,0.9,'eager','resolved','2026-09-27'),('{deep}','employment','payslip',0.9,0.9,'eager','resolved','2026-09-27');
         INSERT INTO job(id,source_id,kind,dedup_key,state) VALUES('job-deep','{deep}','extract_facts','extract:deep','done');
         UPDATE document_evaluation SET state='done';
         DROP TABLE document_fact; DROP TABLE read_summary; PRAGMA user_version=15;")).unwrap();
    drop(vault);
    let vault = crate::Vault::unlock(&root, SYNTHETIC_PASSWORD).unwrap();
    let evaluation = |source: &str| -> (String, i64) {
        vault
            .db
            .query_row(
                "SELECT state,priority FROM document_evaluation WHERE source_id=?",
                [source],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    };
    let (state, priority) = evaluation(&source);
    assert_eq!((state.as_str(), priority), ("queued", 0));
    assert_eq!(evaluation(&deep).0, "done");
    assert_eq!(evaluation(&unclassified).0, "done");
    let version: i64 = vault
        .db
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 17);
    assert_eq!(count(&vault, "SELECT count(*) FROM document_fact"), 0);
}

#[test]
fn a_reread_with_a_changed_amount_leaves_no_conflict() {
    let (mut vault, source) = synthetic_payslip_source();
    let me = vault.profile_entity_id().unwrap();
    let slip = |amount: &str| payslip(&me, vec![money("wage_tax", amount, 10), january()]);
    vault
        .apply_read(&source, &read("a", slip("1032.58")))
        .unwrap();
    let outcome = vault
        .apply_read(&source, &read("b", slip("1035.00")))
        .unwrap();
    let values = accepted(&vault, "person.wage_tax");
    assert_eq!(values.len(), 1);
    assert!(values[0].contains(r#""amount":"1035""#));
    assert_eq!(flagged(&vault, "person.wage_tax"), 0);
    assert_eq!(outcome.checks, 0);
}

#[test]
fn a_reread_with_a_changed_start_leaves_no_stale_interval() {
    let (mut vault, source) = synthetic_payslip_source();
    let me = vault.profile_entity_id().unwrap();
    let notice = |run: &str, number: &str, day: &str| {
        let mut graph = payslip(
            &me,
            vec![
                SlotValue {
                    slot: "tax_number".into(),
                    ..candidate_value(
                        CandidateKind::Identifier,
                        number,
                        CandidateValue::Identifier(number.into()),
                    )
                },
                SlotValue {
                    slot: "document_date".into(),
                    ..date_value(day)
                },
            ],
        );
        graph.doc_type = "tax_assessment".into();
        read(run, graph)
    };
    let intervals = |vault: &Vault| -> Vec<(String, String, Option<String>)> {
        vault
            .db
            .prepare("SELECT value_json,valid_from,valid_to FROM assertion_state WHERE property_key='person.tax_number' AND state='accept'")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap()
    };
    vault
        .apply_read(&source, &notice("a", "21/815/08150", "2020-03-01"))
        .unwrap();
    // A later start: the earlier value of this same document is not kept as a closed interval.
    vault
        .apply_read(&source, &notice("b", "21/815/08151", "2020-03-05"))
        .unwrap();
    let rows = intervals(&vault);
    assert_eq!(rows.len(), 1);
    assert!(rows[0].0.contains("08151"));
    assert_eq!(
        (rows[0].1.as_str(), rows[0].2.as_deref()),
        ("2020-03-05", None)
    );
    // An earlier start: the value it replaces does not close the new value.
    vault
        .apply_read(&source, &notice("c", "21/815/08152", "2020-02-01"))
        .unwrap();
    let rows = intervals(&vault);
    assert_eq!(rows.len(), 1);
    assert!(rows[0].0.contains("08152"));
    assert_eq!(
        (rows[0].1.as_str(), rows[0].2.as_deref()),
        ("2020-02-01", None)
    );
    assert_eq!(conflicts(&vault), 0);
}

#[test]
fn a_reread_keeps_values_other_sources_still_support() {
    let (mut vault, first) = synthetic_payslip_source();
    let me = vault.profile_entity_id().unwrap();
    let certificate = |run: &str, with_id: bool| {
        let mut values = vec![
            money("wage_tax", "12390.96", 10),
            SlotValue {
                slot: "period_start".into(),
                ..date_value("2026-01-01")
            },
            SlotValue {
                slot: "period_end".into(),
                ..date_value("2026-12-31")
            },
        ];
        if with_id {
            values.push(tax_id("12345678903"));
        }
        let mut graph = payslip(&me, values);
        graph.doc_type = "wage_tax_certificate".into();
        read(run, graph)
    };
    vault.apply_read(&first, &certificate("a", true)).unwrap();
    let second = add_synthetic_source(&mut vault, "lstb-copy");
    vault.apply_read(&second, &certificate("b", true)).unwrap();
    // The first certificate no longer states the tax ID; the second still does.
    vault.apply_read(&first, &certificate("c", false)).unwrap();
    assert_eq!(accepted(&vault, "person.tax_id").len(), 1);
    let sources: Vec<String> = vault
        .db
        .prepare("SELECT e.source_id FROM assertion_evidence e JOIN assertion_state a ON a.id=e.assertion_id WHERE a.property_key='person.tax_id' AND a.state='accept'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert_eq!(sources, vec![second.clone()]);
    // Once no source states it, it is retracted.
    vault.apply_read(&second, &certificate("d", false)).unwrap();
    assert!(accepted(&vault, "person.tax_id").is_empty());
    assert_eq!(accepted(&vault, "person.wage_tax").len(), 1);
}

#[test]
fn payslips_first_read_without_a_month_end_without_conflicts_after_rereads() {
    let (mut vault, _) = synthetic_payslip_source();
    let me = vault.profile_entity_id().unwrap();
    // Read without a month first: every amount has an unknown validity and conflicts.
    let mut sources = vec![];
    for m in 1..=12u32 {
        let source = add_synthetic_source(&mut vault, &format!("payslip-{m}"));
        let amount = format!("{}.00", 1000 + m);
        vault
            .apply_read(
                &source,
                &read(
                    &format!("v1-{m}"),
                    payslip(&me, vec![money("wage_tax", &amount, 10)]),
                ),
            )
            .unwrap();
        sources.push(source);
    }
    assert!(flagged(&vault, "person.wage_tax") > 0);
    // Migration 16 reads them again in queue order, now with the pay month.
    for (i, source) in sources.iter().enumerate() {
        let m = i as u32 + 1;
        let last = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][i];
        let amount = format!("{}.00", 1000 + m);
        vault
            .apply_read(
                source,
                &read(
                    &format!("v2-{m}"),
                    payslip(
                        &me,
                        vec![
                            money("wage_tax", &amount, 10),
                            month(&format!("2026-{m:02}-01"), &format!("2026-{m:02}-{last}")),
                        ],
                    ),
                ),
            )
            .unwrap();
    }
    assert_eq!(accepted(&vault, "person.wage_tax").len(), 12);
    assert_eq!(flagged(&vault, "person.wage_tax"), 0);
}

#[test]
fn a_correction_replaces_only_the_value_of_its_own_employer() {
    let (mut vault, acme) = synthetic_payslip_source();
    let me = vault.profile_entity_id().unwrap();
    let slip = |name: &str, values: Vec<SlotValue>| {
        let mut all = vec![employer(name), january()];
        all.extend(values);
        payslip(&me, all)
    };
    vault
        .apply_read(
            &acme,
            &read(
                "x",
                slip("Acme GmbH", vec![money("wage_tax", "500.00", 10)]),
            ),
        )
        .unwrap();
    let beispiel = add_synthetic_source(&mut vault, "payslip-beispiel");
    vault
        .apply_read(
            &beispiel,
            &read(
                "y",
                slip("Beispiel AG", vec![money("wage_tax", "300.00", 10)]),
            ),
        )
        .unwrap();
    let fixed = add_synthetic_source(&mut vault, "correction-acme");
    let mut correction = slip("Acme GmbH", vec![money("wage_tax", "520.00", 10)]);
    correction.correction = true;
    vault.apply_read(&fixed, &read("c", correction)).unwrap();
    let values = accepted(&vault, "person.wage_tax");
    assert_eq!(values.len(), 2);
    assert!(values.iter().any(|v| v.contains(r#""amount":"300""#)));
    assert!(values.iter().any(|v| v.contains(r#""amount":"520""#)));
    assert_eq!(accepted(&vault, "person.employer").len(), 2);
    // A re-read of the correction no longer finds its amount: the other employer's
    // value does not keep the correction standing, so the original comes back.
    let mut without = slip("Acme GmbH", vec![]);
    without.correction = true;
    vault.apply_read(&fixed, &read("c2", without)).unwrap();
    vault
        .apply_read(
            &acme,
            &read(
                "x2",
                slip("Acme GmbH", vec![money("wage_tax", "500.00", 10)]),
            ),
        )
        .unwrap();
    let values = accepted(&vault, "person.wage_tax");
    assert_eq!(values.len(), 2);
    assert!(values.iter().any(|v| v.contains(r#""amount":"300""#)));
    assert!(values.iter().any(|v| v.contains(r#""amount":"500""#)));
}

#[test]
fn a_correction_without_an_employer_leaves_a_conflict_to_check() {
    let (mut vault, source) = synthetic_payslip_source();
    let me = vault.profile_entity_id().unwrap();
    vault
        .apply_read(
            &source,
            &read(
                "o",
                payslip(&me, vec![money("wage_tax", "1032.58", 10), january()]),
            ),
        )
        .unwrap();
    let fixed = add_synthetic_source(&mut vault, "correction");
    let mut correction = payslip(&me, vec![money("wage_tax", "1040.00", 10), january()]);
    correction.correction = true;
    vault.apply_read(&fixed, &read("c", correction)).unwrap();
    assert_eq!(accepted(&vault, "person.wage_tax").len(), 2);
    assert_eq!(flagged(&vault, "person.wage_tax"), 2);
}

#[test]
fn a_correction_replaces_only_single_valued_period_values() {
    let (mut vault, earlier) = synthetic_payslip_source();
    let me = vault.profile_entity_id().unwrap();
    vault
        .apply_read(
            &earlier,
            &read(
                "e",
                payslip(
                    &me,
                    vec![
                        employer("Acme GmbH"),
                        money("wage_tax", "500.00", 10),
                        tax_id("12345678903"),
                        january(),
                    ],
                ),
            ),
        )
        .unwrap();
    // The earlier payslip also names a second employer for January (an additive
    // graph): a many-valued value of a source from the correcting employer.
    vault
        .apply_document_graph(
            &earlier,
            &payslip(&me, vec![employer("Beispiel AG"), january()]),
        )
        .unwrap();
    assert_eq!(accepted(&vault, "person.employer").len(), 2);
    let fixed = add_synthetic_source(&mut vault, "correction");
    let mut correction = payslip(
        &me,
        vec![
            employer("Acme GmbH"),
            money("wage_tax", "520.00", 10),
            tax_id("98765432106"),
            january(),
        ],
    );
    correction.correction = true;
    vault.apply_read(&fixed, &read("c", correction)).unwrap();
    let wage = accepted(&vault, "person.wage_tax");
    assert_eq!(wage.len(), 1);
    assert!(wage[0].contains(r#""amount":"520""#));
    // Links to several employers and timeless identifiers are not replaced.
    assert_eq!(accepted(&vault, "person.employer").len(), 2);
    assert_eq!(accepted(&vault, "person.tax_id").len(), 2);
    assert_eq!(flagged(&vault, "person.tax_id"), 2);
}

#[test]
fn a_correction_leaves_a_value_the_user_confirmed() {
    let (mut vault, source) = synthetic_payslip_source();
    let me = vault.profile_entity_id().unwrap();
    vault
        .apply_read(
            &source,
            &read(
                "o",
                payslip(
                    &me,
                    vec![
                        employer("Acme GmbH"),
                        money("wage_tax", "1032.58", 10),
                        january(),
                    ],
                ),
            ),
        )
        .unwrap();
    let confirmed: String = vault
        .db
        .query_row(
            "SELECT id FROM assertion_state WHERE property_key='person.wage_tax' AND state='accept'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    vault.confirm_assertion(&confirmed).unwrap();
    let fixed = add_synthetic_source(&mut vault, "correction");
    let mut correction = payslip(
        &me,
        vec![
            employer("Acme GmbH"),
            money("wage_tax", "1040.00", 10),
            january(),
        ],
    );
    correction.correction = true;
    vault.apply_read(&fixed, &read("c", correction)).unwrap();
    assert_eq!(accepted(&vault, "person.wage_tax").len(), 2);
    assert_eq!(
        vault.review_state(&confirmed).unwrap(),
        crate::ReviewState::Reviewed
    );
    let state: String = vault
        .db
        .query_row(
            "SELECT state FROM assertion_state WHERE id=?",
            [&confirmed],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "accept");
}

#[test]
fn read_checks_count_only_accepted_values_of_this_source() {
    let (mut vault, source) = synthetic_payslip_source();
    let me = vault.profile_entity_id().unwrap();
    let mut unsure = money("wage_tax", "1032.58", 10);
    unsure.confidence = 0.75;
    let first = vault
        .apply_read(&source, &read("a", payslip(&me, vec![unsure, january()])))
        .unwrap();
    assert_eq!(first.checks, 1);
    // The re-read no longer finds the unsure value: nothing is left to check.
    let second = vault
        .apply_read(&source, &read("b", payslip(&me, vec![january()])))
        .unwrap();
    assert_eq!(second.checks, 0);
    assert_eq!(
        count(&vault, "SELECT checks FROM read_summary WHERE run_id='b'"),
        0
    );
}

#[test]
fn a_read_with_an_unknown_owner_or_period_is_rejected() {
    let (mut vault, source) = synthetic_payslip_source();
    let mut owner = fact("Kostenstelle", "4711", None);
    owner.owner = "employer".into();
    let mut period = fact("Lohnsteuer", "1.032,58", None);
    period.period = Some("monthly".into());
    for bad in [owner, period] {
        let result = vault.apply_read(
            &source,
            &DocumentRead {
                run_id: "r".into(),
                facts: vec![fact("Steuerklasse", "1", None), bad],
                ..Default::default()
            },
        );
        assert!(matches!(result, Err(crate::Error::Validation(_))));
    }
    assert_eq!(count(&vault, "SELECT count(*) FROM document_fact"), 0);
    assert_eq!(count(&vault, "SELECT count(*) FROM read_summary"), 0);
}

type Snapshot = (
    Vec<(String, String, String, Option<String>)>,
    Vec<(String, String)>,
    i64,
);
/// Profile values, their evidence and the decision count, for before/after checks.
fn profile_snapshot(vault: &Vault) -> Snapshot {
    let states = vault
        .db
        .prepare("SELECT id,state,valid_from,valid_to FROM assertion_state ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    let evidence = vault
        .db
        .prepare(
            "SELECT assertion_id,source_id FROM assertion_evidence ORDER BY assertion_id,source_id",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    (
        states,
        evidence,
        count(vault, "SELECT count(*) FROM decision"),
    )
}
fn unverified(label: &str, value: &str) -> ReadFact {
    ReadFact {
        owner: "unknown".into(),
        period: None,
        state: FactState::Unverified,
        confidence: None,
        ..fact(label, value, None)
    }
}

#[test]
fn an_unverified_read_keeps_profile_values_and_their_evidence() {
    let (mut vault, source) = synthetic_payslip_source();
    let me = vault.profile_entity_id().unwrap();
    let mut unsure = money("church_tax", "10.00", 30);
    unsure.confidence = 0.75;
    let verified = DocumentRead {
        run_id: "a".into(),
        graph: Some(payslip(
            &me,
            vec![money("wage_tax", "1032.58", 10), unsure, january()],
        )),
        facts: vec![
            fact("Lohnsteuer", "1.032,58", Some("wage_tax")),
            fact("Kostenstelle", "4711", None),
        ],
        ..Default::default()
    };
    vault.apply_read(&source, &verified).unwrap();
    let before = profile_snapshot(&vault);
    assert!(before.0.iter().any(|(_, state, _, _)| state == "accept"));
    assert!(!before.1.is_empty());

    // Verification failed on a re-read: only the document's facts and summary change.
    let read = DocumentRead {
        run_id: "b".into(),
        graph: None,
        facts: vec![unverified("Lohnsteuer", "1.032,58")],
        uninterpreted: vec![crate::Uncovered {
            segment_id: "seg".into(),
            start: 40,
            end: 46,
            kind: CandidateKind::Amount,
            label: Some("KV-Beitrag".into()),
            text: "435,21".into(),
            line: "KV-Beitrag 435,21".into(),
        }],
        rejected: [("value_not_in_quote".to_owned(), 1)].into(),
    };
    let outcome = vault.store_unverified_read(&source, &read).unwrap();
    assert_eq!(
        outcome,
        ReadOutcome {
            values: 1,
            in_profile: 0,
            // The source's standing check is still there.
            checks: 1,
            uninterpreted: 1
        }
    );
    assert_eq!(profile_snapshot(&vault), before);
    assert_eq!(accepted(&vault, "person.wage_tax").len(), 1);
    assert_eq!(
        count(
            &vault,
            "SELECT count(*) FROM decision WHERE reason_code='not_found_on_reread'"
        ),
        0
    );
    assert_eq!(
        count(
            &vault,
            "SELECT count(*) FROM document_fact WHERE run_id<>'b'"
        ),
        0
    );
    assert_eq!(
        count(
            &vault,
            "SELECT count(*) FROM document_fact WHERE run_id='b' AND state IN ('unverified','uninterpreted') AND owner='unknown' AND assertion_id IS NULL"
        ),
        2
    );
    assert_eq!(
        count(
            &vault,
            "SELECT count(*) FROM read_summary WHERE run_id='b' AND values_read=1 AND in_profile=0 AND uninterpreted=1"
        ),
        1
    );

    // Only unverified, unlinked facts and no profile values are stored this way.
    let mut checked = read.clone();
    checked.facts[0].state = FactState::Verified;
    let mut linked = read.clone();
    linked.facts[0].slot = Some("wage_tax".into());
    let mut valued = read.clone();
    valued.graph = Some(payslip(&me, vec![january()]));
    for bad in [checked, linked, valued] {
        assert!(matches!(
            vault.store_unverified_read(&source, &bad),
            Err(crate::Error::Validation(_))
        ));
    }
    assert_eq!(profile_snapshot(&vault), before);
    assert_eq!(
        count(&vault, "SELECT count(*) FROM read_summary WHERE run_id='b'"),
        1
    );
}

#[test]
fn a_verified_read_without_profile_values_withdraws_the_sources_automatic_values() {
    // The counterpart of an unverified read: a verified read of a type that fills
    // no profile values (graph None) replaces this source's automatic values, so
    // values it no longer supports are withdrawn. A person's decision stays.
    let (mut vault, source) = synthetic_payslip_source();
    let me = vault.profile_entity_id().unwrap();
    let first = DocumentRead {
        run_id: "a".into(),
        graph: Some(payslip(
            &me,
            vec![
                money("wage_tax", "1032.58", 10),
                money("church_tax", "10.00", 30),
                january(),
            ],
        )),
        ..Default::default()
    };
    vault.apply_read(&source, &first).unwrap();
    let church: String = vault
        .db
        .query_row(
            "SELECT id FROM assertion_state WHERE property_key='person.church_tax' AND state='accept'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    vault.confirm_assertion(&church).unwrap();
    let verified = DocumentRead {
        run_id: "b".into(),
        graph: None,
        facts: vec![fact("Kostenstelle", "4711", None)],
        ..Default::default()
    };
    let outcome = vault.apply_read(&source, &verified).unwrap();
    assert_eq!((outcome.values, outcome.in_profile), (1, 0));
    assert!(accepted(&vault, "person.wage_tax").is_empty());
    assert_eq!(accepted(&vault, "person.church_tax").len(), 1);
    assert!(
        count(
            &vault,
            "SELECT count(*) FROM decision WHERE actor='policy' AND action='retract' AND reason_code='not_found_on_reread'"
        ) >= 1
    );
}

#[test]
fn the_document_view_groups_profile_values_other_details_and_uninterpreted_values() {
    let (mut vault, source) = synthetic_payslip_source();
    let item = vault.source_item(&source).unwrap().unwrap();
    let me = vault.profile_entity_id().unwrap();
    let mut uncertain = fact("Kostenstelle", "4711", None);
    uncertain.state = FactState::Uncertain;
    let read = DocumentRead {
        run_id: "r".into(),
        graph: Some(payslip(
            &me,
            vec![
                money("wage_tax", "1032.58", 10),
                month("2026-01-01", "2026-01-31"),
            ],
        )),
        facts: vec![fact("Lohnsteuer", "1.032,58", Some("wage_tax")), uncertain],
        uninterpreted: vec![crate::Uncovered {
            segment_id: "seg".into(),
            start: 40,
            end: 46,
            kind: CandidateKind::Amount,
            label: Some("KV-Beitrag".into()),
            text: "435,21".into(),
            line: "KV-Beitrag 435,21".into(),
        }],
        rejected: Default::default(),
    };
    vault.apply_read(&source, &read).unwrap();
    let view = vault.document_read(item).unwrap();
    assert_eq!(view.in_profile.len(), 1);
    assert_eq!(view.in_profile[0].label, "Lohnsteuer");
    assert!(view.in_profile[0].assertion.is_some());
    assert_eq!(view.other.len(), 1);
    assert!(view.other[0].uncertain);
    assert_eq!(view.uninterpreted[0].value, "435,21");
}

#[test]
fn the_document_view_shows_checks_locations_and_periods_and_follows_decisions() {
    let (mut vault, source) = synthetic_payslip_source();
    let item = vault.source_item(&source).unwrap().unwrap();
    let me = vault.profile_entity_id().unwrap();
    vault
        .db
        .execute(
            "INSERT INTO source_segment(id,source_id,extraction_revision,ordinal,locator_json,text,content_hash) VALUES('seg-p1',?,1,1,?,'Lohnsteuer 1.032,58',?)",
            params![
                source,
                json!({"kind":"document_text","method":"ocr","page":1,"section":null}).to_string(),
                hash(b"Lohnsteuer 1.032,58")
            ],
        )
        .unwrap();
    let mut unsure = money("wage_tax", "1032.58", 10);
    unsure.confidence = 0.75;
    let mut wage_tax = fact("Lohnsteuer", "1.032,58", Some("wage_tax"));
    wage_tax.segment_id = "seg-p1".into();
    wage_tax.context = "Januar 2026".into();
    let read = DocumentRead {
        run_id: "r".into(),
        graph: Some(payslip(&me, vec![unsure, january()])),
        facts: vec![wage_tax, fact("Kostenstelle", "4711", None)],
        ..Default::default()
    };
    vault.apply_read(&source, &read).unwrap();

    // A linked value below the check threshold is in the profile and waits in Quick checks.
    let view = vault.document_read(item).unwrap();
    assert_eq!(
        view.in_profile,
        vec![ReadRow {
            label: "Lohnsteuer".into(),
            value: "1.032,58".into(),
            period: Some("Januar 2026".into()),
            location: "Page 1 · OCR".into(),
            uncertain: false,
            assertion: view.in_profile[0].assertion.clone(),
            check: true,
        }]
    );
    let assertion = view.in_profile[0].assertion.clone().unwrap();
    // Facts without a known segment keep the generic location and no period.
    assert_eq!(view.other[0].location, "Source");
    assert_eq!(view.other[0].period, None);
    assert!(!view.other[0].check);
    assert!(view.uninterpreted.is_empty());

    // "Looks right" ends the check; the value stays in the profile.
    vault.confirm_assertion(&assertion).unwrap();
    let view = vault.document_read(item).unwrap();
    assert_eq!(view.in_profile.len(), 1);
    assert!(!view.in_profile[0].check);

    // A value the person rejected is no longer in the profile: the fact stays with
    // its document as another detail.
    vault.reject_assertion(&assertion).unwrap();
    let view = vault.document_read(item).unwrap();
    assert!(view.in_profile.is_empty());
    assert_eq!(view.other.len(), 2);
    assert!(view.other.iter().all(|r| r.assertion.is_none() && !r.check));

    // Unknown items show nothing.
    assert_eq!(
        vault.document_read(item + 1000).unwrap(),
        DocumentReadView::default()
    );
}
