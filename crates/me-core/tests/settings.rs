use me_core::{AppSettings, Content, DocumentClass, ExtractionOutput, Vault};
use std::path::Path;
const PASSWORD: &str = "synthetic-test-passphrase";
fn import(v: &mut Vault, dir: &Path, name: &str, class: DocumentClass) -> u64 {
    let path = dir.join(name);
    std::fs::write(&path, "SYNTHETIC Erika Beispiel\nGeburtsdatum: 01.02.1990").unwrap();
    v.import_document(&path, name, class).unwrap()
}
fn state(v: &Vault, item: u64) -> String {
    match &v.collection("", false).unwrap().get(item).unwrap().content {
        Content::Document { state, .. } => state.clone(),
        _ => panic!("Expected document"),
    }
}
#[test]
fn automatic_default_opt_out_and_manual_release_survive_restart() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vault");
    let mut v = Vault::create(&root, PASSWORD).unwrap();
    assert_eq!(v.settings().unwrap(), AppSettings::default());
    let first = import(
        &mut v,
        temp.path(),
        "first.pdf",
        DocumentClass::Unclassified,
    );
    assert_eq!(v.next_automatic_document().unwrap(), Some(first));
    v.set_automatic_evaluation(false).unwrap();
    assert!(!v.begin_evaluation(first, true).unwrap());
    let second = import(
        &mut v,
        temp.path(),
        "second.jpg",
        DocumentClass::Unclassified,
    );
    drop(v);
    let mut v = Vault::unlock(&root, PASSWORD).unwrap();
    assert!(!v.settings().unwrap().automatic_evaluation);
    assert_eq!(state(&v, first), "manual");
    assert_eq!(state(&v, second), "manual");
    assert_eq!(v.next_automatic_document().unwrap(), None);
    assert!(v.begin_evaluation(second, false).unwrap());
    assert!(v.begin_evaluation(second, false).is_err());
    v.finish_evaluation(second, Some("synthetic failure"))
        .unwrap();
    v.set_automatic_evaluation(true).unwrap();
    assert_eq!(v.next_automatic_document().unwrap(), Some(first));
    assert_eq!(
        v.evaluation_error(second).unwrap().as_deref(),
        Some("synthetic failure")
    );
}
#[test]
fn queue_skips_credentials_unsupported_files_and_failed_or_finished_work() {
    let temp = tempfile::tempdir().unwrap();
    let mut v = Vault::create(&temp.path().join("vault"), PASSWORD).unwrap();
    let secret = import(&mut v, temp.path(), "secret.pdf", DocumentClass::Credential);
    let unsupported = import(
        &mut v,
        temp.path(),
        "table.xlsx",
        DocumentClass::Unclassified,
    );
    let first = import(
        &mut v,
        temp.path(),
        "first.pdf",
        DocumentClass::Unclassified,
    );
    let second = import(&mut v, temp.path(), "second.txt", DocumentClass::Personal);
    assert!(v.begin_evaluation(secret, false).is_err());
    assert!(v.begin_evaluation(unsupported, false).is_err());
    assert_eq!(v.next_automatic_document().unwrap(), Some(first));
    assert!(v.begin_evaluation(first, true).unwrap());
    v.finish_evaluation(first, Some("synthetic failure"))
        .unwrap();
    assert_eq!(v.next_automatic_document().unwrap(), Some(second));
    let input = v.prepare_extraction(second, "synthetic").unwrap();
    v.finish_extraction(&input, ExtractionOutput { facts: vec![] })
        .unwrap();
    assert_eq!(v.next_automatic_document().unwrap(), None);
    assert!(v.begin_evaluation(first, false).unwrap());
}
#[test]
fn interrupted_work_waits_for_explicit_resume_even_when_automation_is_enabled() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vault");
    let mut v = Vault::create(&root, PASSWORD).unwrap();
    let item = import(&mut v, temp.path(), "one.pdf", DocumentClass::Unclassified);
    assert!(v.begin_evaluation(item, true).unwrap());
    drop(v);
    let mut v = Vault::unlock(&root, PASSWORD).unwrap();
    assert_eq!(v.next_automatic_document().unwrap(), None);
    assert_eq!(state(&v, item), "failed");
    assert!(v.begin_evaluation(item, false).unwrap());
    v.set_automatic_evaluation(false).unwrap();
    drop(v);
    let v = Vault::unlock(&root, PASSWORD).unwrap();
    assert_eq!(v.next_automatic_document().unwrap(), None);
    assert_eq!(state(&v, item), "failed");
}
