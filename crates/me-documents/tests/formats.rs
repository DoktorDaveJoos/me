use me_documents::extract;
use std::sync::atomic::AtomicBool;

#[test]
fn text_formats_preserve_values_and_reject_binary_or_oversized_input() {
    let cancel = AtomicBool::new(false);
    for ext in ["txt", "MD", "csv", "tsv", "log"] {
        let parts = extract(
            ext,
            "\u{feff}SYNTHETIC München; 01234567890".as_bytes(),
            &cancel,
        )
        .unwrap();
        assert_eq!(parts[0].text, "SYNTHETIC München; 01234567890");
    }
    assert!(extract("txt", b"binary\0data", &cancel).is_err());
    assert!(extract("csv", &vec![b'x'; 1_048_577], &cancel).is_err());
    assert!(extract("exe", b"data", &cancel).is_err());
    assert!(extract("txt", b"   ", &cancel).is_err());
    assert!(extract("txt", b"data", &AtomicBool::new(true)).is_err());
}

#[cfg(target_os = "macos")]
fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

#[test]
#[cfg(target_os = "macos")]
fn native_pdf_scan_and_office_extraction_retains_page_evidence() {
    let cancel = AtomicBool::new(false);
    for (ext, name) in [
        ("pdf", "text.pdf"),
        ("pdf", "scan.pdf"),
        ("pdf", "mixed.pdf"),
        ("rtf", "sample.rtf"),
    ] {
        let parts = extract(ext, &fixture(name), &cancel).unwrap();
        assert!(
            parts.iter().any(|p| p.text.contains("01234567890")),
            "{name}"
        );
        assert!(
            parts.iter().any(|p| p.text.contains("01.02.1990")),
            "{name}"
        );
        if name == "text.pdf" {
            assert!(
                parts
                    .iter()
                    .any(|p| p.method == "pdf_text" && p.page == Some(1))
            );
        }
        if name == "scan.pdf" {
            assert!(parts.iter().all(|p| p.method == "ocr" && p.page == Some(1)));
        }
        if name == "mixed.pdf" {
            assert!(parts.iter().any(|p| p.method == "ocr" && p.page == Some(2)));
        }
    }
}

#[test]
#[cfg(target_os = "macos")]
fn photos_and_scans_include_exif_orientation() {
    let cancel = AtomicBool::new(false);
    for name in [
        "scan.jpg",
        "scan.png",
        "scan.tiff",
        "scan.bmp",
        "scan.gif",
        "scan.heic",
        "scan.webp",
        "rotated.jpg",
    ] {
        let ext = name.rsplit('.').next().unwrap();
        let parts = extract(ext, &fixture(name), &cancel).unwrap();
        assert!(
            parts.iter().any(|p| p.text.contains("01234567890")),
            "{name}"
        );
        assert!(parts.iter().all(|p| p.method == "ocr"));
    }
}

#[test]
#[cfg(target_os = "macos")]
fn damaged_locked_and_large_pdfs_fail_without_partial_results() {
    let cancel = AtomicBool::new(false);
    assert!(extract("pdf", b"%PDF-broken", &cancel).is_err());
    assert!(
        extract("pdf", &fixture("locked.pdf"), &cancel)
            .unwrap_err()
            .contains("password protected")
    );
    assert!(
        extract("pdf", &fixture("over-page-limit.pdf"), &cancel)
            .unwrap_err()
            .contains("500 pages")
    );
    assert!(extract("jpg", b"not an image", &cancel).is_err());
}

#[test]
#[cfg(target_os = "macos")]
fn an_imported_scan_reaches_the_inbox_model_input_with_its_page() {
    let temp = tempfile::tempdir().unwrap();
    let mut vault =
        me_core::Vault::create(&temp.path().join("vault"), "synthetic-test-passphrase").unwrap();
    let file = temp.path().join("Bescheinigung.pdf");
    std::fs::write(&file, fixture("scan.pdf")).unwrap();
    let item = vault
        .import_document(
            &file,
            "synthetic-delivery",
            me_core::DocumentClass::Unclassified,
        )
        .unwrap();
    let document = vault.begin_document_processing(item).unwrap().unwrap();
    let parts = extract(
        &document.extension,
        &document.bytes,
        &AtomicBool::new(false),
    )
    .unwrap();
    vault.finish_document_processing(&document, &parts).unwrap();
    let input = vault.prepare_extraction(item, "synthetic-model").unwrap();
    let segment = input
        .segments
        .iter()
        .find(|s| s.text.contains("01234567890"))
        .unwrap();
    let evidence = vault
        .evidence_read(&vault.shareable_scope().unwrap(), &segment.segment_id)
        .unwrap();
    assert_eq!(evidence["locator"]["page"], 1);
    assert_eq!(evidence["locator"]["method"], "ocr");
    assert_eq!(
        vault.collection("01234567890", false).unwrap().items[0].id,
        item
    );
}

#[test]
#[cfg(target_os = "macos")]
fn sixty_page_pdf_reports_progress_and_retains_the_last_page() {
    let mut progress = Vec::new();
    let parts = me_documents::extract_with_progress(
        "pdf",
        &fixture("large.pdf"),
        &AtomicBool::new(false),
        &mut |page, total| progress.push((page, total)),
    )
    .unwrap();
    assert_eq!(progress.last(), Some(&(60, 60)));
    assert!(progress.windows(2).all(|w| w[1].0 > w[0].0));
    assert!(
        parts
            .iter()
            .any(|p| p.page == Some(60) && p.text.contains("01234 567890"))
    );
    assert!(parts.iter().map(|p| p.text.len()).sum::<usize>() > 24_000);
}

#[test]
#[cfg(target_os = "macos")]
fn payroll_columns_keep_labels_and_values_together() {
    let cancel = AtomicBool::new(false);
    for name in ["payroll.pdf", "payroll-scan.pdf", "payroll.jpg"] {
        let parts = extract(name.rsplit('.').next().unwrap(), &fixture(name), &cancel).unwrap();
        let text = parts
            .iter()
            .map(|p| p.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        for value in [
            "0004711",
            "01.02.1990",
            "4.500,00",
            "2.900,00",
            "2.800,00",
            "COBADEFFXXX",
        ] {
            assert!(text.contains(value), "{name} lost a payroll value");
        }
        if name != "payroll.pdf" {
            assert!(
                text.lines()
                    .any(|line| line.contains("Auszahlungsbetrag") && line.contains("2.800,00")),
                "{name} lost row alignment: {text}"
            );
        }
    }
}

#[cfg(target_os = "macos")]
#[test]
fn empty_pdf_widgets_are_available_for_form_matching() {
    let parts = me_documents::extract(
        "pdf",
        include_bytes!("fixtures/form.pdf"),
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    let text = parts
        .iter()
        .map(|p| p.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Empty form field: tax_id"));
    assert!(text.contains("Empty form field: birth_date"));
    assert!(text.contains("Steuer-ID"));
}
