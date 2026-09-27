use me_core::{DocumentClass, ExtractedFact, ExtractionOutput, Vault};

#[test]
fn checkpoints_survive_restart_are_encrypted_and_reject_stale_or_unproven_writes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vault");
    let file = temp.path().join("source.txt");
    std::fs::write(&file, "Erika Beispiel\nSteuer-ID: 01234567890").unwrap();
    let mut v = Vault::create(&root, "synthetic-password").unwrap();
    let item = v
        .import_document(&file, "synthetic", DocumentClass::Personal)
        .unwrap();
    v.set_automatic_evaluation(false).unwrap();
    assert!(v.begin_evaluation(item, false).unwrap());
    let first = v.prepare_extraction(item, "synthetic").unwrap();
    let output = ExtractionOutput {
        facts: vec![ExtractedFact {
            property: "person.tax_id".into(),
            value: "01234567890".into(),
            segment_id: first.segments[0].segment_id.clone(),
            quote: "01234567890".into(),
            subject_quote: "Erika Beispiel".into(),
            context_quote: String::new(),
            slot: String::new(),
        }],
    };
    let mut bad = output.clone();
    bad.facts[0].quote = "invented".into();
    assert!(v.save_extraction_checkpoint(&first, "v1", &bad).is_err());
    v.save_extraction_checkpoint(&first, "v1", &output).unwrap();
    assert!(v.proposals(item).unwrap().is_empty());
    v.fail_extraction(&first.run_id).unwrap();
    v.finish_evaluation(item, Some("synthetic interruption"))
        .unwrap();
    assert!(v.save_extraction_checkpoint(&first, "v1", &output).is_err());
    drop(v);
    let database = std::fs::read(root.join("vault.db")).unwrap();
    assert!(!database.windows(11).any(|w| w == b"01234567890"));
    let mut v = Vault::unlock(&root, "synthetic-password").unwrap();
    assert!(!v.settings().unwrap().automatic_evaluation);
    assert!(v.begin_evaluation(item, false).unwrap());
    let second = v.prepare_extraction(item, "synthetic").unwrap();
    assert_ne!(first.run_id, second.run_id);
    assert!(v.extraction_checkpoint(&second, "v2").unwrap().is_none());
    let saved = v.extraction_checkpoint(&second, "v1").unwrap().unwrap();
    assert_eq!(v.finish_extraction(&second, saved).unwrap(), 1);
    v.finish_evaluation_with_warning(item, None, Some("One unsupported candidate was excluded"))
        .unwrap();
    assert!(v.evaluation_warning(item).unwrap().is_some());
    // A warning permits a manual retry without destroying the good proposals.
    assert!(v.begin_evaluation(item, false).unwrap());
    let third = v.prepare_extraction(item, "synthetic").unwrap();
    let saved = v.extraction_checkpoint(&third, "v1").unwrap().unwrap();
    v.finish_extraction(&third, saved).unwrap();
    v.finish_evaluation(item, None).unwrap();
    assert!(v.evaluation_warning(item).unwrap().is_none());
    assert_eq!(v.proposals(item).unwrap().len(), 1);
    assert!(v.begin_evaluation(item, false).unwrap());
    let fresh = v.prepare_extraction(item, "synthetic").unwrap();
    assert!(v.extraction_checkpoint(&fresh, "v1").unwrap().is_none());
    assert_eq!(v.proposals(item).unwrap().len(), 1);
}

#[test]
fn a_checkpoint_keeps_values_whose_only_open_question_is_ownership() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("source.txt");
    std::fs::write(&file, "Steuer-ID: 01234567890\nGeburtsdatum: 01.02.1990").unwrap();
    let mut v = Vault::create(&temp.path().join("vault"), "synthetic-password").unwrap();
    let item = v
        .import_document(&file, "synthetic", DocumentClass::Personal)
        .unwrap();
    v.set_automatic_evaluation(false).unwrap();
    assert!(v.begin_evaluation(item, false).unwrap());
    let input = v.prepare_extraction(item, "synthetic").unwrap();
    let unowned = ExtractedFact {
        property: "person.tax_id".into(),
        value: "01234567890".into(),
        segment_id: input.segments[0].segment_id.clone(),
        quote: "Steuer-ID: 01234567890".into(),
        subject_quote: String::new(),
        context_quote: String::new(),
        slot: "tax_id".into(),
    };
    let output = ExtractionOutput {
        facts: vec![unowned.clone()],
    };
    v.save_extraction_checkpoint(&input, "v1", &output).unwrap();
    let saved = v.extraction_checkpoint(&input, "v1").unwrap().unwrap();
    assert_eq!(saved.facts.len(), 1);
    assert!(saved.facts[0].subject_quote.is_empty());
    assert_eq!(saved.facts[0].slot, "tax_id");
    let restored = me_core::ground_extraction(&input, saved);
    assert!(restored.output.facts.is_empty());
    assert_eq!(restored.rejected.len(), 1);
    assert_eq!(restored.rejected[0].code, "subject_unknown");
    // Any other open question still keeps the whole section out of the checkpoint.
    let mut invented = unowned;
    invented.quote = "Steuer-ID: 99999999999".into();
    let bad = ExtractionOutput {
        facts: vec![output.facts[0].clone(), invented],
    };
    assert!(v.save_extraction_checkpoint(&input, "v2", &bad).is_err());
    assert!(v.extraction_checkpoint(&input, "v2").unwrap().is_none());
}
