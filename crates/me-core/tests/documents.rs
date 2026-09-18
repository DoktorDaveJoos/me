use me_core::{DocumentClass, DocumentPart, ExtractedFact, ExtractionOutput, Vault};
const PASSWORD: &str = "synthetic-document-passphrase";
fn part(text: &str, page: u32) -> DocumentPart {
    DocumentPart {
        text: text.into(),
        page: Some(page),
        section: None,
        method: "ocr".into(),
    }
}
#[test]
fn extracted_pdf_is_searchable_with_page_provenance_and_reviewable_facts() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vault");
    let mut vault = Vault::create(&root, PASSWORD).unwrap();
    let file = temp.path().join("scan.PDF");
    std::fs::write(&file, b"synthetic source bytes").unwrap();
    let item = vault
        .import_document(&file, "one", DocumentClass::Unclassified)
        .unwrap();
    assert!(
        vault
            .collection("01234567890", false)
            .unwrap()
            .items
            .is_empty()
    );
    let doc = vault.begin_document_processing(item).unwrap().unwrap();
    assert_eq!(&*doc.bytes, b"synthetic source bytes");
    assert!(vault.begin_document_processing(item).is_err());
    vault
        .finish_document_processing(
            &doc,
            &[part("SYNTHETIC Erika Beispiel\nSteuer-ID: 01234567890", 2)],
        )
        .unwrap();
    assert!(vault.begin_document_processing(item).unwrap().is_none());
    assert_eq!(
        vault.collection("01234567890", false).unwrap().items.len(),
        1
    );
    let input = vault.prepare_extraction(item, "synthetic-model").unwrap();
    let segment = input.segments[0].segment_id.clone();
    let evidence = vault
        .evidence_read(&vault.shareable_scope().unwrap(), &segment)
        .unwrap();
    assert_eq!(evidence["locator"]["page"], 2);
    assert_eq!(evidence["locator"]["method"], "ocr");
    vault
        .finish_extraction(
            &input,
            ExtractionOutput {
                facts: vec![ExtractedFact {
                    property: "person.tax_id".into(),
                    value: "01234567890".into(),
                    segment_id: segment.clone(),
                    quote: "Steuer-ID: 01234567890".into(),
                    subject_quote: "Erika Beispiel".into(),
                    context_quote: String::new(),
                }],
            },
        )
        .unwrap();
    assert_eq!(
        vault.proposals(item).unwrap()[0].location_label,
        "Page 2 · OCR"
    );
    vault.review_proposals(item, true).unwrap();
    drop(vault);
    let vault = Vault::unlock(&root, PASSWORD).unwrap();
    let evidence = vault
        .evidence_read(&vault.shareable_scope().unwrap(), &segment)
        .unwrap();
    assert_eq!(evidence["locator"]["page"], 2);
    for entry in std::fs::read_dir(&root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            assert!(
                !std::fs::read(path)
                    .unwrap()
                    .windows(11)
                    .any(|w| w == b"01234567890")
            );
        }
    }
}
#[test]
fn failed_cancelled_or_interrupted_jobs_can_retry_and_reject_stale_completions() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vault");
    let mut vault = Vault::create(&root, PASSWORD).unwrap();
    let file = temp.path().join("photo.jpg");
    std::fs::write(&file, b"synthetic source bytes").unwrap();
    let item = vault
        .import_document(&file, "one", DocumentClass::Unclassified)
        .unwrap();
    let secret = vault
        .import_document(&file, "secret", DocumentClass::Credential)
        .unwrap();
    assert!(vault.begin_document_processing(secret).is_err());
    let first = vault.begin_document_processing(item).unwrap().unwrap();
    assert!(
        vault
            .finish_document_processing(&first, &[part("", 1)])
            .is_err()
    );
    vault.fail_document_processing(&first).unwrap();
    let second = vault.begin_document_processing(item).unwrap().unwrap();
    // An unrelated import must not replay an OCR job that is currently outside the mutex.
    let other = temp.path().join("unrelated.txt");
    std::fs::write(&other, "SYNTHETIC another source").unwrap();
    vault
        .import_document(&other, "parallel-import", DocumentClass::Personal)
        .unwrap();
    assert!(vault.begin_document_processing(item).is_err());

    assert!(
        vault
            .finish_document_processing(&first, &[part("stale", 1)])
            .is_err()
    );
    // Simulate app exit mid-OCR. Unlock makes the interrupted job retryable.
    drop(vault);
    let mut vault = Vault::unlock(&root, PASSWORD).unwrap();
    let third = vault.begin_document_processing(item).unwrap().unwrap();
    assert!(
        vault
            .finish_document_processing(&second, &[part("stale", 1)])
            .is_err()
    );
    assert!(
        vault
            .finish_document_processing(
                &third,
                &[part("too many pages", me_core::MAX_DOCUMENT_PAGES + 1)]
            )
            .is_err()
    );
    assert!(vault.collection("stale", false).unwrap().items.is_empty());
    vault
        .finish_document_processing(&third, &[part("SYNTHETIC Wiederaufnahme München", 1)])
        .unwrap();
    assert_eq!(
        vault
            .collection("Wiederaufnahme", false)
            .unwrap()
            .items
            .len(),
        1
    );
}

#[test]
fn long_document_has_no_24kb_gate_and_validates_all_sections_atomically() {
    let temp = tempfile::tempdir().unwrap();
    let mut vault = Vault::create(&temp.path().join("vault"), PASSWORD).unwrap();
    let file = temp.path().join("long.pdf");
    std::fs::write(&file, b"synthetic PDF bytes").unwrap();
    let item = vault
        .import_document(&file, "synthetic", DocumentClass::Unclassified)
        .unwrap();
    assert!(vault.begin_evaluation(item, true).unwrap());
    let document = vault.begin_document_processing(item).unwrap().unwrap();
    let text = format!(
        "Erika Beispiel\nSteuer-ID: 01234567890\n{}",
        "ü".repeat(700)
    );
    let parts: Vec<_> = (1..=26).map(|page| part(&text, page)).collect();
    vault.finish_document_processing(&document, &parts).unwrap();
    let input = vault.prepare_extraction(item, "synthetic").unwrap();
    assert!(input.segments.iter().map(|s| s.text.len()).sum::<usize>() > 24_000);
    let facts: Vec<_> = input
        .segments
        .iter()
        .filter(|s| s.text.contains("01234567890"))
        .map(|s| ExtractedFact {
            property: "person.tax_id".into(),
            value: "01234567890".into(),
            segment_id: s.segment_id.clone(),
            quote: "Steuer-ID: 01234567890".into(),
            subject_quote: "Erika Beispiel".into(),
            context_quote: String::new(),
        })
        .collect();
    assert_eq!(facts.len(), 26);
    let mut invalid = facts.clone();
    invalid.last_mut().unwrap().quote = "Invented 01234567890".into();
    assert!(
        vault
            .finish_extraction(&input, ExtractionOutput { facts: invalid })
            .is_err()
    );
    assert!(vault.proposals(item).unwrap().is_empty());
    assert_eq!(
        vault
            .finish_extraction(&input, ExtractionOutput { facts })
            .unwrap(),
        26
    );
    vault.finish_evaluation(item, None).unwrap();
    assert_eq!(vault.proposals(item).unwrap().len(), 26);
    assert!(vault.next_automatic_document().unwrap().is_none());
}
