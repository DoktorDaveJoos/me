use me_core::{DocumentClass, ExtractedFact, ExtractionOutput, Vault, normalize_fact};
const PASSWORD: &str = "synthetic-test-passphrase";
fn facts(v: &Vault) -> serde_json::Value {
    v.facts_get(
        &v.shareable_scope().unwrap(),
        &["person.tax_id".into(), "person.birth_date".into()],
    )
    .unwrap()
}
#[test]
fn typed_values_are_exact_and_conflicts_do_not_pick_a_winner() {
    let t = tempfile::tempdir().unwrap();
    let mut v = Vault::create(&t.path().join("v"), PASSWORD).unwrap();
    v.save_note(None, "Steuer-ID", "01234 567890").unwrap();
    v.save_note(None, "Geburtsdatum", "29.02.2000").unwrap();
    assert_eq!(facts(&v)["facts"][0]["value"], "01234567890");
    assert_eq!(facts(&v)["facts"][1]["value"], "2000-02-29");
    assert!(v.save_note(None, "Geburtsdatum", "29.02.1900").is_err());
    v.save_note(None, "Steuer-ID", "11234567890").unwrap();
    assert_eq!(facts(&v)["facts"][0]["status"], "conflicting");
    assert!(facts(&v)["facts"][0]["value"].is_null());
    assert!(normalize_fact("person.tax_id", "12/345/67890").is_err());
}
#[test]
fn scope_is_a_snapshot_and_credential_and_unclassified_text_stay_private() {
    let t = tempfile::tempdir().unwrap();
    let mut v = Vault::create(&t.path().join("v"), PASSWORD).unwrap();
    v.save_note(None, "Notiz", "SYNTHETIC shared").unwrap();
    let scope = v.shareable_scope().unwrap();
    v.save_note(None, "Steuer-ID", "01234567890").unwrap();
    assert_eq!(
        v.facts_get(&scope, &["person.tax_id".into()]).unwrap()["facts"][0]["status"],
        "missing"
    );
    let file = t.path().join("SECRET.txt");
    std::fs::write(&file, "SECRET synthetic").unwrap();
    v.import_document(&file, "a", DocumentClass::Credential)
        .unwrap();
    v.import_document(&file, "b", DocumentClass::Unclassified)
        .unwrap();
    assert!(
        v.documents_search(&v.shareable_scope().unwrap(), "SECRET", 10)
            .unwrap()["hits"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let shared = v.documents_search(&scope, "shared", 10).unwrap();
    let segment = shared["hits"][0]["segment_id"].as_str().unwrap();
    assert!(v.evidence_read(&scope, segment).is_ok());
    assert!(v.evidence_read(&Default::default(), segment).is_err());
}
fn document(v: &mut Vault, t: &std::path::Path) -> u64 {
    let f = t.join("letter.txt");
    std::fs::write(
        &f,
        "SYNTHETIC Musterperson\nSteuer-ID: 01234567890\nGeburtsdatum: 01.02.1990",
    )
    .unwrap();
    v.import_document(&f, "letter", DocumentClass::Personal)
        .unwrap()
}
#[test]
fn proposals_require_real_evidence_and_explicit_ownership_confirmation() {
    let t = tempfile::tempdir().unwrap();
    let mut v = Vault::create(&t.path().join("v"), PASSWORD).unwrap();
    let item = document(&mut v, t.path());
    let input = v.prepare_extraction(item, "synthetic-model").unwrap();
    let mut fact = ExtractedFact {
        property: "person.tax_id".into(),
        value: "01234567890".into(),
        segment_id: input.segments[0].segment_id.clone(),
        quote: "Invented 01234567890".into(),
        subject_quote: "SYNTHETIC Musterperson".into(),
        context_quote: String::new(),
    };
    assert!(
        v.finish_extraction(
            &input,
            ExtractionOutput {
                facts: vec![fact.clone()]
            }
        )
        .is_err()
    );
    assert!(v.proposals(item).unwrap().is_empty());
    fact.quote = "Steuer-ID: 01234567890".into();
    assert_eq!(
        v.finish_extraction(
            &input,
            ExtractionOutput {
                facts: vec![fact.clone()]
            }
        )
        .unwrap(),
        1
    );
    assert_eq!(facts(&v)["facts"][0]["status"], "unverified");
    assert!(
        v.finish_extraction(&input, ExtractionOutput { facts: vec![fact] })
            .is_err()
    );
    v.review_proposals(item, true).unwrap();
    assert_eq!(facts(&v)["facts"][0]["status"], "resolved");
    assert_eq!(facts(&v)["facts"][0]["value"], "01234567890");
    assert!(v.proposals(item).unwrap().is_empty());
    drop(v);
    let v = Vault::unlock(&t.path().join("v"), PASSWORD).unwrap();
    assert_eq!(facts(&v)["facts"][0]["value"], "01234567890");
}
#[test]
fn interrupted_extraction_can_resume_without_duplicate_intake() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("v");
    let mut v = Vault::create(&root, PASSWORD).unwrap();
    let item = document(&mut v, t.path());
    v.prepare_extraction(item, "synthetic-model").unwrap();
    drop(v);
    let mut v = Vault::unlock(&root, PASSWORD).unwrap();
    let input = v.prepare_extraction(item, "synthetic-model").unwrap();
    v.finish_extraction(&input, ExtractionOutput { facts: vec![] })
        .unwrap();
    assert_eq!(v.collection("", false).unwrap().total, 1);
}
