use me_core::{DocumentClass, FormNeed, Vault, filter_data};
#[test]
fn english_and_german_intent_returns_values_without_answers() {
    let tmp = tempfile::tempdir().unwrap();
    let mut vault = Vault::create(&tmp.path().join("vault"), "synthetic-password").unwrap();
    vault.save_note(None, "Steuer-ID", "01234567890").unwrap();
    vault.save_note(None, "Geburtsdatum", "01.02.1990").unwrap();
    let facts = vault.data_facts().unwrap();
    for query in [
        "I want to have my Steuer ID",
        "tax ID",
        "Steu",
        "meine Steueridentifikationsnummer",
    ] {
        let rows = filter_data(&facts, query, &[]);
        assert_eq!(rows.len(), 1, "{query}");
        assert_eq!(rows[0].value, "01234567890");
    }
    assert!(filter_data(&facts, "my passport number", &[]).is_empty());
    assert_eq!(
        filter_data(&facts, "", &["person.birth_date".into()]).len(),
        1
    );
    assert!(filter_data(&facts, "", &["invented".into()]).is_empty());
}
#[test]
fn semantic_fields_extend_local_results_without_duplicates() {
    let tmp = tempfile::tempdir().unwrap();
    let mut vault = Vault::create(&tmp.path().join("vault"), "synthetic-password").unwrap();
    vault
        .save_note(None, "Instagram reminder", "Synthetic reference")
        .unwrap();
    vault.save_note(None, "Geburtsdatum", "01.02.1990").unwrap();
    let facts = vault.data_facts().unwrap();
    let matches = filter_data(&facts, "Instagram", &["person.birth_date".into()]);
    assert_eq!(matches.len(), 2);
    assert!(
        matches
            .iter()
            .any(|fact| fact.label == "Instagram reminder")
    );
    let matches = filter_data(&facts, "Geburtsdatum", &["person.birth_date".into()]);
    assert_eq!(matches.len(), 1);
    assert_eq!(
        filter_data(&facts, "Instagram", &["invented".into()]).len(),
        1
    );
}
#[test]
fn recents_persist_but_retracted_values_and_unknown_ids_do_not() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("vault");
    let mut vault = Vault::create(&path, "synthetic-password").unwrap();
    let item = vault.save_note(None, "Steuer-ID", "01234567890").unwrap();
    let fact = vault.data_facts().unwrap().remove(0);
    assert!(
        vault
            .remember_data(&[fact.id.clone(), "invented".into()])
            .is_err()
    );
    assert_eq!(vault.data_facts().unwrap()[0].recent, 0);
    vault.remember_data(&[fact.id]).unwrap();
    drop(vault);
    let mut vault = Vault::unlock(&path, "synthetic-password").unwrap();
    assert!(vault.data_facts().unwrap()[0].recent > 0);
    vault
        .save_note(Some(item), "Steuer-ID", "11111111111")
        .unwrap();
    let rows = vault.data_facts().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].value, "11111111111");
    assert_eq!(rows[0].recent, 0);
}
#[test]
fn form_handoff_exports_only_unambiguous_confirmed_matches() {
    let tmp = tempfile::tempdir().unwrap();
    let mut vault = Vault::create(&tmp.path().join("vault"), "synthetic-password").unwrap();
    vault.save_note(None, "Steuer-ID", "01234567890").unwrap();
    vault
        .save_note(None, "Sozialversicherungsnummer", "12010290B123")
        .unwrap();
    vault
        .save_note(None, "Sozialversicherungsnummer", "22010290B123")
        .unwrap();
    vault
        .save_note(None, "Unrelated private note", "not for this form")
        .unwrap();
    let file = tmp.path().join("form.txt");
    std::fs::write(&file, "Steuer-ID: ____\nSV-Nummer: ____\nSignature: ____").unwrap();
    let item = vault
        .import_document(&file, "synthetic", DocumentClass::Personal)
        .unwrap();
    let needs = vec![
        FormNeed {
            label: "Tax ID".into(),
            keys: vec!["person.tax_id".into()],
            quote: "Steuer-ID".into(),
        },
        FormNeed {
            label: "Social insurance".into(),
            keys: vec!["person.social_insurance_number".into()],
            quote: "SV-Nummer".into(),
        },
        FormNeed {
            label: "Signature".into(),
            keys: vec![],
            quote: "Signature".into(),
        },
    ];
    let folder = vault
        .export_form_handoff(&[item], &needs, tmp.path())
        .unwrap();
    let text = std::fs::read_to_string(folder.join("matched-data.json")).unwrap();
    assert!(text.contains("01234567890"));
    assert!(!text.contains("12010290B123"));
    assert!(!text.contains("22010290B123"));
    assert!(!text.contains("not for this form"));
    assert!(text.contains("needs_choice"));
    assert!(text.contains("missing"));
    assert_eq!(
        std::fs::read(folder.join("form-1.txt")).unwrap(),
        std::fs::read(file).unwrap()
    );
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&folder).unwrap().permissions().mode() & 0o777,
        0o700
    );
}
