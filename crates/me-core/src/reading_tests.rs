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
fn count(vault: &Vault, sql: &str) -> i64 {
    vault.db.query_row(sql, [], |r| r.get(0)).unwrap()
}
fn conflicts(vault: &Vault) -> i64 {
    count(
        vault,
        "SELECT count(*) FROM assertion_review WHERE check_reason='conflict'",
    )
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
    let (vault, source, root) = synthetic_vault_on_disk();
    // A version 15 vault: the legacy-fixture helper is not used because it removes
    // the version 15 tables (document_profile, priority) that this migration reads.
    vault.db.execute_batch(&format!(
        "INSERT INTO document_profile(source_id,family,doc_type,family_confidence,type_confidence,tier,graph_state,classified_at) VALUES('{source}','employment','payslip',0.9,0.9,'eager','resolved','2026-09-27');
         UPDATE document_evaluation SET state='done' WHERE source_id='{source}';
         DROP TABLE document_fact; DROP TABLE read_summary; PRAGMA user_version=15;")).unwrap();
    drop(vault);
    let vault = crate::Vault::unlock(&root, SYNTHETIC_PASSWORD).unwrap();
    let (state, priority): (String, i64) = vault
        .db
        .query_row(
            "SELECT state,priority FROM document_evaluation WHERE source_id=?",
            [&source],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((state.as_str(), priority), ("queued", 0));
    let version: i64 = vault
        .db
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 16);
    assert_eq!(count(&vault, "SELECT count(*) FROM document_fact"), 0);
}
