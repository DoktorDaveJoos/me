use me_core::{
    DocumentClass, ExtractedFact, ExtractionOutput, ImportErrorKind, ImportFailure, ImportProvider,
    ImportStage, Vault,
};
use serde_json::json;
const PASSWORD: &str = "synthetic-recovery-passphrase";
fn document(v: &mut Vault, dir: &std::path::Path, name: &str) -> u64 {
    let file = dir.join(name);
    std::fs::write(&file, "Erika Beispiel\nSteuer-ID: 01234567890").unwrap();
    v.import_document(&file, name, DocumentClass::Personal)
        .unwrap()
}
#[test]
fn accepted_atomic_facts_never_appear_as_separate_imports() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = Vault::create(&dir.path().join("vault"), PASSWORD).unwrap();
    let item = document(&mut v, dir.path(), "letter.txt");
    let input = v.prepare_extraction(item, "synthetic").unwrap();
    v.finish_extraction(
        &input,
        ExtractionOutput {
            facts: vec![ExtractedFact {
                property: "person.tax_id".into(),
                value: "01234567890".into(),
                segment_id: input.segments[0].segment_id.clone(),
                quote: "01234567890".into(),
                subject_quote: "Erika Beispiel".into(),
                context_quote: String::new(),
            }],
        },
    )
    .unwrap();
    v.review_proposals(item, true).unwrap();
    assert!(v.collection("", false).unwrap().total > 1);
    let jobs = v.import_jobs().unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].item, item);
}
#[test]
fn reservations_and_step_results_survive_restart_without_resetting_allowance() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("vault");
    let mut v = Vault::create(&root, PASSWORD).unwrap();
    let item = document(&mut v, dir.path(), "letter.txt");
    v.begin_evaluation(item, false).unwrap();
    v.begin_import_progress(item, "first").unwrap();
    let first = v.prepare_extraction(item, "synthetic").unwrap();
    for _ in 0..12 {
        v.reserve_import_request(&first, ImportProvider::OpenAi)
            .unwrap();
    }
    assert!(
        v.reserve_import_request(&first, ImportProvider::OpenAi)
            .is_err()
    );
    let call = v
        .reserve_import_request(&first, ImportProvider::TypeSafe)
        .unwrap();
    assert!(
        v.record_import_usage(item, "first", &call, 100, 25)
            .unwrap()
    );
    assert!(
        v.record_import_usage(item, "first", &call, 100, 25)
            .unwrap()
    );
    assert_eq!(v.import_usage(item).unwrap().input_tokens, 100);
    v.save_import_step_cache(
        &first,
        "test-v1",
        "extract",
        &json!({"untrusted":"01234567890"}),
    )
    .unwrap();
    v.update_import_progress(item, "first", ImportStage::Extracting, 2, 4)
        .unwrap();
    assert!(v.proposals(item).unwrap().is_empty());
    drop(v);
    let bytes = std::fs::read(root.join("vault.db")).unwrap();
    assert!(!bytes.windows(11).any(|w| w == b"01234567890"));
    let mut v = Vault::unlock(&root, PASSWORD).unwrap();
    assert_eq!(v.import_jobs().unwrap()[0].state, "failed");
    assert_eq!(v.next_automatic_document().unwrap(), None);
    assert_eq!(v.import_steps(item).unwrap()[3].current, 2);
    assert!(
        !v.record_import_usage(item, "first", &call, 9000, 9000)
            .unwrap()
    );
    v.begin_evaluation(item, false).unwrap();
    v.begin_import_progress(item, "second").unwrap();
    let second = v.prepare_extraction(item, "synthetic").unwrap();
    assert_eq!(
        v.import_step_cache(&second, "test-v1", "extract")
            .unwrap()
            .unwrap()["untrusted"],
        "01234567890"
    );
    assert!(
        v.reserve_import_request(&second, ImportProvider::OpenAi)
            .is_err()
    );
    assert!(
        v.reserve_import_request(&first, ImportProvider::TypeSafe)
            .is_err()
    );
    assert!(
        !v.finish_import_evaluation(item, "first", None, None)
            .unwrap()
    );
    assert!(
        !v.record_import_failure(
            item,
            "first",
            &ImportFailure::new(ImportProvider::OpenAi, ImportErrorKind::Quota, "stale")
        )
        .unwrap()
    );
    assert!(v.import_pause_reason().unwrap().is_none());
    assert!(
        !v.update_import_progress(item, "first", ImportStage::Complete, 1, 1)
            .unwrap()
    );
    for _ in 1..24 {
        v.reserve_import_request(&second, ImportProvider::TypeSafe)
            .unwrap();
    }
    assert!(
        v.reserve_import_request(&second, ImportProvider::TypeSafe)
            .is_err()
    );
    assert!(v.extend_import_allowance(item).is_err());
    v.fail_extraction(&second.run_id).unwrap();
    v.finish_import_evaluation(item, "second", Some("Allowance reached"), None)
        .unwrap();
    let usage = v.import_usage(item).unwrap();
    assert_eq!(
        (
            usage.openai_calls,
            usage.typesafe_calls,
            usage.unreported_calls
        ),
        (12, 24, 35)
    );
    v.extend_import_allowance(item).unwrap();
    let usage = v.import_usage(item).unwrap();
    assert_eq!((usage.openai_limit, usage.typesafe_limit), (24, 48));
    assert_eq!(usage.openai_calls, 12);
    v.begin_evaluation(item, false).unwrap();
    v.begin_import_progress(item, "third").unwrap();
    let third = v.prepare_extraction(item, "synthetic").unwrap();
    v.reserve_import_request(&third, ImportProvider::OpenAi)
        .unwrap();
}
#[test]
fn provider_quota_pauses_other_files_persistently_until_explicit_resume() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("vault");
    let mut v = Vault::create(&root, PASSWORD).unwrap();
    let a = document(&mut v, dir.path(), "a.txt");
    let b = document(&mut v, dir.path(), "b.txt");
    v.begin_evaluation(a, true).unwrap();
    v.begin_import_progress(a, "a").unwrap();
    v.update_import_progress(a, "a", ImportStage::Extracting, 0, 2)
        .unwrap();
    let failure = ImportFailure::new(
        ImportProvider::OpenAi,
        ImportErrorKind::Quota,
        "OpenAI usage limit reached.",
    );
    v.record_import_failure(a, "a", &failure).unwrap();
    v.finish_import_evaluation(a, "a", Some(&failure.message), None)
        .unwrap();
    assert_eq!(v.next_automatic_document().unwrap(), None);
    assert!(v.begin_evaluation(b, true).is_err());
    drop(v);
    let mut v = Vault::unlock(&root, PASSWORD).unwrap();
    assert!(v.import_pause_reason().unwrap().is_some());
    let job = v
        .import_jobs()
        .unwrap()
        .into_iter()
        .find(|j| j.item == a)
        .unwrap();
    assert_eq!(job.stage, ImportStage::Extracting);
    assert_eq!(job.failure.unwrap().kind, ImportErrorKind::Quota);
    v.resume_import_queue().unwrap();
    assert_eq!(v.next_automatic_document().unwrap(), Some(b));
}
#[test]
fn reported_token_cap_prevents_the_next_request_and_does_not_double_count() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = Vault::create(&dir.path().join("vault"), PASSWORD).unwrap();
    let item = document(&mut v, dir.path(), "a.txt");
    v.begin_evaluation(item, false).unwrap();
    v.begin_import_progress(item, "a").unwrap();
    let input = v.prepare_extraction(item, "synthetic").unwrap();
    let call = v
        .reserve_import_request(&input, ImportProvider::OpenAi)
        .unwrap();
    v.record_import_usage(item, "a", &call, 170_000, 10_000)
        .unwrap();
    v.record_import_usage(item, "a", &call, 100, 100).unwrap();
    assert_eq!(v.import_usage(item).unwrap().input_tokens, 170_000);
    assert!(
        v.reserve_import_request(&input, ImportProvider::OpenAi)
            .is_err()
    );
    assert!(
        v.reserve_import_request(&input, ImportProvider::TypeSafe)
            .is_err()
    );
}
