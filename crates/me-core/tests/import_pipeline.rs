use me_core::{DocumentClass, ImportStage, Vault};
const PASSWORD: &str = "synthetic-import-pipeline-passphrase";
fn add(vault: &mut Vault, dir: &std::path::Path, name: &str) -> u64 {
    let path = dir.join(name);
    std::fs::write(&path, "SYNTHETIC\nPolicy: 00042").unwrap();
    vault
        .import_document(&path, name, DocumentClass::Unclassified)
        .unwrap()
}
#[test]
fn parallel_claims_skip_active_documents_and_keep_failures_independent() {
    let dir = tempfile::tempdir().unwrap();
    let mut vault = Vault::create(&dir.path().join("vault"), PASSWORD).unwrap();
    let a = add(&mut vault, dir.path(), "a.txt");
    let b = add(&mut vault, dir.path(), "b.eml");
    assert_eq!(
        vault.next_automatic_document_excluding(&[a]).unwrap(),
        Some(b)
    );
    assert!(vault.begin_evaluation(a, true).unwrap());
    assert!(!vault.begin_evaluation(a, true).unwrap());
    assert_eq!(vault.next_automatic_document().unwrap(), Some(b));
    assert!(vault.begin_evaluation(b, true).unwrap());
    vault
        .finish_evaluation(a, Some("Synthetic failure"))
        .unwrap();
    assert_eq!(
        vault
            .import_jobs()
            .unwrap()
            .iter()
            .find(|j| j.item == b)
            .unwrap()
            .state,
        "running"
    );
    vault.set_automatic_evaluation(false).unwrap();
    assert_eq!(vault.next_automatic_document().unwrap(), None);
}
#[test]
fn progress_survives_restart_and_stale_workers_cannot_overwrite_a_retry() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("vault");
    let mut vault = Vault::create(&root, PASSWORD).unwrap();
    let item = add(&mut vault, dir.path(), "a.txt");
    vault.begin_evaluation(item, true).unwrap();
    vault.begin_import_progress(item, "first").unwrap();
    assert!(
        vault
            .update_import_progress(item, "first", ImportStage::Verifying, 2, 3)
            .unwrap()
    );
    drop(vault);
    let mut vault = Vault::unlock(&root, PASSWORD).unwrap();
    let jobs = vault.import_jobs().unwrap();
    assert_eq!(jobs[0].state, "failed");
    assert_eq!(jobs[0].stage, ImportStage::Verifying);
    assert_eq!((jobs[0].current, jobs[0].total), (2, 3));
    assert!(
        !vault
            .update_import_progress(item, "first", ImportStage::Complete, 0, 0)
            .unwrap()
    );
    vault.begin_evaluation(item, false).unwrap();
    vault.begin_import_progress(item, "retry").unwrap();
    assert!(
        !vault
            .update_import_progress(item, "first", ImportStage::Complete, 0, 0)
            .unwrap()
    );
    assert!(
        vault
            .update_import_progress(item, "retry", ImportStage::Extracting, 1, 3)
            .unwrap()
    );
    vault
        .finish_evaluation(item, Some("Synthetic cancellation"))
        .unwrap();
    assert!(
        !vault
            .update_import_progress(item, "retry", ImportStage::Complete, 0, 0)
            .unwrap()
    );
    let job = &vault.import_jobs().unwrap()[0];
    assert_eq!(job.stage, ImportStage::Extracting);
    assert!(job.error.is_some());
}

#[test]
fn search_form_intake_is_reserved_for_explicit_form_processing() {
    let dir = tempfile::tempdir().unwrap();
    let mut vault = Vault::create(&dir.path().join("vault"), PASSWORD).unwrap();
    let form = add(&mut vault, dir.path(), "form.txt");
    let document = add(&mut vault, dir.path(), "letter.txt");
    vault.defer_automatic_document(form).unwrap();
    assert_eq!(vault.next_automatic_document().unwrap(), Some(document));
    assert!(!vault.begin_evaluation(form, true).unwrap());
    assert!(vault.begin_evaluation(form, false).unwrap());
}
