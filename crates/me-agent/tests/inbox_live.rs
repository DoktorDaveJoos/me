//! Explicit live regression: only synthetic fixtures, never the user's vault.
#[test]
#[cfg(target_os = "macos")]
#[ignore = "Requires ME_CODEX_TEST_HOME pointing to a signed-in ME. Codex home; uses synthetic GPT-5.6 Sol calls"]
fn pdf_scan_and_jpeg_reach_reviewable_proposals_through_the_real_cli() {
    use me_core::{DocumentClass, Vault};
    use std::{
        path::PathBuf,
        sync::{Arc, atomic::AtomicBool},
    };
    let home = PathBuf::from(
        std::env::var_os("ME_CODEX_TEST_HOME").expect("Explicit test Codex home required"),
    );
    let temp = tempfile::tempdir().unwrap();
    let mut vault = Vault::create(&temp.path().join("vault"), "synthetic-test-passphrase").unwrap();
    for file in ["text.pdf", "scan.pdf", "scan.jpg"] {
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../me-documents/tests/fixtures")
            .join(file);
        let item = vault
            .import_document(&source, file, DocumentClass::Unclassified)
            .unwrap();
        assert_eq!(vault.next_automatic_document().unwrap(), Some(item));
        assert!(vault.begin_evaluation(item, true).unwrap());
        let document = vault.begin_document_processing(item).unwrap().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let parts = me_documents::extract(&document.extension, &document.bytes, &cancel).unwrap();
        vault.finish_document_processing(&document, &parts).unwrap();
        let input = vault
            .prepare_extraction(item, me_agent::codex::INBOX_MODEL)
            .unwrap();
        let result = me_agent::codex::extract(&home, &input, cancel, |_| {}).unwrap();
        assert!(
            vault.finish_extraction(&input, result).unwrap() >= 2,
            "Expected synthetic birth date and tax ID"
        );
        vault.finish_evaluation(item, None).unwrap();
        let proposals = vault.proposals(item).unwrap();
        assert!(proposals.iter().any(|p| p.value == "01234567890"));
        assert!(proposals.iter().any(|p| p.value == "1990-02-01"));
        assert_eq!(vault.next_automatic_document().unwrap(), None);
        let facts = vault
            .facts_get(&vault.shareable_scope().unwrap(), &["person.tax_id".into()])
            .unwrap();
        assert_eq!(facts["facts"][0]["status"], "unverified");
    }
}

#[test]
#[cfg(target_os = "macos")]
#[ignore = "Requires signed-in ME_CODEX_TEST_HOME; synthetic long PDF, GPT-5.6 Sol calls"]
fn long_pdf_finds_facts_beyond_the_former_limit() {
    use me_core::{DocumentClass, Vault};
    use std::{
        path::PathBuf,
        sync::{Arc, atomic::AtomicBool},
    };
    let home =
        PathBuf::from(std::env::var_os("ME_CODEX_TEST_HOME").expect("Explicit test home required"));
    let temp = tempfile::tempdir().unwrap();
    me_diagnostics::init(temp.path().join("logs"), "synthetic-long-pdf-test");
    let mut vault = Vault::create(&temp.path().join("vault"), "synthetic-test-passphrase").unwrap();
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../me-documents/tests/fixtures/long.pdf");
    let item = vault
        .import_document(&path, "Synthetic long PDF", DocumentClass::Unclassified)
        .unwrap();
    assert!(vault.begin_evaluation(item, true).unwrap());
    let document = vault.begin_document_processing(item).unwrap().unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let parts = me_documents::extract(&document.extension, &document.bytes, &cancel).unwrap();
    let preceding: usize = parts.iter().take(11).map(|p| p.text.len()).sum();
    assert!(preceding > 24_000, "Facts must occur after the old limit");
    vault.finish_document_processing(&document, &parts).unwrap();
    let input = vault
        .prepare_extraction(item, me_agent::codex::INBOX_MODEL)
        .unwrap();
    let mut second_section = false;
    let output = me_agent::codex::extract(&home, &input, cancel, |event| {
        if let me_agent::codex::Progress::Message(message) = event {
            second_section |= message.contains("Section 2 of");
            println!("{message}");
        }
    })
    .unwrap();
    assert!(second_section);
    assert!(vault.finish_extraction(&input, output).unwrap() >= 2);
    vault.finish_evaluation(item, None).unwrap();
    let proposals = vault.proposals(item).unwrap();
    assert!(
        proposals
            .iter()
            .any(|p| p.value == "01234567890" && p.location_label.contains("12"))
    );
    assert!(proposals.iter().any(|p| p.value == "1990-02-01"));
    let report = me_diagnostics::report();
    assert!(report.contains("codex.batch.finished"));
    for private in [
        "Erika",
        "01234567890",
        "1990-02-01",
        "Synthetic long PDF",
        "segment_id",
        "authToken",
    ] {
        assert!(!report.contains(private));
    }
    println!("Verified long PDF, page 12 evidence and metadata-only diagnostics.");
}

#[test]
#[cfg(target_os = "macos")]
#[ignore = "Requires signed-in ME_CODEX_TEST_HOME; synthetic sixty-page PDF and persisted checkpoints"]
fn large_pdf_reaches_page_sixty_and_reuses_verified_sections() {
    use me_core::{DocumentClass, ExtractionInput, ExtractionOutput, Vault};
    use std::{
        path::PathBuf,
        sync::{Arc, atomic::AtomicBool},
    };
    struct Store<'a>(&'a mut Vault);
    impl me_agent::codex::Checkpoints for Store<'_> {
        fn load(&mut self, input: &ExtractionInput) -> Result<Option<ExtractionOutput>, String> {
            self.0
                .extraction_checkpoint(input, me_agent::codex::PIPELINE)
                .map_err(|e| e.to_string())
        }
        fn save(
            &mut self,
            input: &ExtractionInput,
            output: &ExtractionOutput,
        ) -> Result<(), String> {
            self.0
                .save_extraction_checkpoint(input, me_agent::codex::PIPELINE, output)
                .map_err(|e| e.to_string())
        }
    }
    let home =
        PathBuf::from(std::env::var_os("ME_CODEX_TEST_HOME").expect("Explicit test home required"));
    let temp = tempfile::tempdir().unwrap();
    let mut v = Vault::create(&temp.path().join("vault"), "synthetic-passphrase").unwrap();
    let source =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../me-documents/tests/fixtures/large.pdf");
    let item = v
        .import_document(&source, "synthetic-large", DocumentClass::Unclassified)
        .unwrap();
    v.begin_evaluation(item, true).unwrap();
    let document = v.begin_document_processing(item).unwrap().unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut last_page = 0;
    let parts = me_documents::extract_with_progress(
        &document.extension,
        &document.bytes,
        &cancel,
        &mut |page, _| last_page = page,
    )
    .unwrap();
    assert_eq!(last_page, 60);
    v.finish_document_processing(&document, &parts).unwrap();
    let input = v
        .prepare_extraction(item, me_agent::codex::INBOX_MODEL)
        .unwrap();
    let report = me_agent::codex::extract_document(
        &home,
        &input,
        cancel.clone(),
        |p| {
            if let me_agent::codex::Progress::Message(s) = p {
                println!("{s}");
            }
        },
        &mut Store(&mut v),
    )
    .unwrap();
    assert_eq!(report.rejected.len(), 0);
    let mut resumed = 0;
    let replay=me_agent::codex::extract_document(&home,&input,cancel,|p|{if matches!(p,me_agent::codex::Progress::Message(ref s) if s.contains("restoring saved results")){resumed+=1;}},&mut Store(&mut v)).unwrap();
    assert!(resumed >= 3);
    assert_eq!(report.output.facts.len(), replay.output.facts.len());
    assert!(v.finish_extraction(&input, replay.output).unwrap() >= 2);
    v.finish_evaluation(item, None).unwrap();
    let proposals = v.proposals(item).unwrap();
    assert!(
        proposals
            .iter()
            .any(|p| p.value == "01234567890" && p.location_label.contains("60"))
    );
    assert!(proposals.iter().any(|p| p.value == "1990-02-01"));
    println!("Verified 60-page PDF and encrypted checkpoint reuse.");
}

#[test]
#[cfg(target_os = "macos")]
#[ignore = "Uses the signed-in ME_CODEX_TEST_HOME and synthetic payslips with GPT-5.6 Sol"]
fn payroll_pdf_scan_and_jpeg_extract_full_document() {
    use me_core::{DocumentClass, Vault};
    use std::{
        path::PathBuf,
        sync::{Arc, atomic::AtomicBool},
    };
    let home =
        PathBuf::from(std::env::var_os("ME_CODEX_TEST_HOME").expect("Explicit test home required"));
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../me-documents/tests/fixtures");
    let expected: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(root.join("payroll.json")).unwrap()).unwrap();
    let compact = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    for file in ["payroll.pdf", "payroll-scan.pdf", "payroll.jpg"] {
        if std::env::var("ME_PAYROLL_TEST_FORMAT").is_ok_and(|selected| selected != file) {
            continue;
        }
        let started = std::time::Instant::now();
        let temp = tempfile::tempdir().unwrap();
        let mut v = Vault::create(&temp.path().join("vault"), "synthetic-password").unwrap();
        let item = v
            .import_document(&root.join(file), file, DocumentClass::Unclassified)
            .unwrap();
        v.begin_evaluation(item, true).unwrap();
        let document = v.begin_document_processing(item).unwrap().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let parts = me_documents::extract(&document.extension, &document.bytes, &cancel).unwrap();
        v.finish_document_processing(&document, &parts).unwrap();
        let input = v
            .prepare_extraction(item, me_agent::codex::INBOX_MODEL)
            .unwrap();
        let report = me_agent::codex::extract_document(
            &home,
            &input,
            cancel,
            |p| {
                if let me_agent::codex::Progress::Message(s) = p {
                    println!("{file}: {s}");
                }
            },
            &mut me_agent::codex::NoCheckpoints,
        )
        .unwrap();
        println!(
            "{file}: {} grounded facts, {} questions, {} seconds",
            report.output.facts.len(),
            report.rejected.len(),
            started.elapsed().as_secs()
        );
        if let Some(dir) = std::env::var_os("ME_PAYROLL_TEST_REPORT_DIR") {
            let path = PathBuf::from(dir).join(format!("{file}.json"));
            std::fs::write(path,serde_json::to_vec_pretty(&serde_json::json!({"output":report.output,"rejected":report.rejected.iter().map(|r|serde_json::json!({"fact":r.fact,"reason":r.code})).collect::<Vec<_>>()})).unwrap()).unwrap();
        }
        let missing: Vec<_> = expected
            .iter()
            .filter(|(_, value)| {
                !report
                    .output
                    .facts
                    .iter()
                    .any(|f| compact(&f.value) == compact(value.as_str().unwrap()))
            })
            .map(|(k, _)| k)
            .collect();
        // Every printed value must survive the full OCR/model/evidence path.
        assert!(
            missing.is_empty(),
            "{file}: missing synthetic fields {missing:?}"
        );
        assert!(
            report.output.facts.len() >= 30,
            "Expected all 30 payroll fields, not only identity data"
        );
        assert!(
            report
                .output
                .facts
                .iter()
                .any(|f| f.property == "person.birth_date" && f.value == "01.02.1990")
        );
        assert!(
            report.output.facts.iter().any(
                |f| f.value.contains("2.900,00") && f.property.to_lowercase().contains("netto")
            )
        );
        assert!(
            report
                .output
                .facts
                .iter()
                .any(|f| f.value.contains("2.800,00")
                    && f.property.to_lowercase().contains("auszahl"))
        );
        assert!(
            report
                .output
                .facts
                .iter()
                .filter(|f| f.value.contains("EUR"))
                .all(|f| f.context_quote.contains("2026"))
        );
        let count = v
            .finish_extraction_with_questions(&input, report.output, report.rejected)
            .unwrap();
        v.finish_evaluation(item, None).unwrap();
        assert!(count >= 30);
        assert!(
            v.proposals(item)
                .unwrap()
                .iter()
                .any(|p| p.label.contains("August 2026"))
        );
        v.review_proposals(item, true).unwrap();
        assert!(!v.collection("2.800", false).unwrap().items.is_empty());
        println!(
            "{file}: all 30 fixture fields reached persistent review and searchable saved facts."
        );
    }
}
