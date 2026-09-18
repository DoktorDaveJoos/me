use me_core::{
    DocumentClass, ExtractedFact, ExtractionOutput, SourceText, Vault, ground_extraction,
};

fn candidate(segment: &str, property: &str, value: &str, period: &str) -> ExtractedFact {
    ExtractedFact {
        property: property.into(),
        value: value.into(),
        segment_id: segment.into(),
        quote: value.into(),
        subject_quote: "Erika Beispiel".into(),
        context_quote: period.into(),
    }
}

#[test]
fn arbitrary_document_fields_survive_review_restart_and_period_separation() {
    let t = tempfile::tempdir().unwrap();
    let file = t.path().join("payroll.txt");
    std::fs::write(&file,"Erika Beispiel\nAugust 2026\nSeptember 2026\nBrutto: 4.500,00 EUR\nNetto: 2.900,00 EUR\nAuszahlung: 2.800,00 EUR\nPersonalnummer: 0004711\nIBAN: DE89 3704 0044 0532 0130 00\nBank: Musterbank\nAbzug: -100,00 EUR\nEintritt: 01.03.2020\n").unwrap();
    let root = t.path().join("vault");
    let mut v = Vault::create(&root, "synthetic-passphrase").unwrap();
    let item = v
        .import_document(&file, "Gehaltsabrechnungen", DocumentClass::Personal)
        .unwrap();
    let input = v.prepare_extraction(item, "synthetic-model").unwrap();
    let segment = &input.segments[0].segment_id;
    let mut facts = vec![
        candidate(
            segment,
            "document.Gesamtbrutto",
            "4.500,00 EUR",
            "August 2026",
        ),
        candidate(
            segment,
            "document.Gesamtbrutto",
            "4.500,00 EUR",
            "September 2026",
        ),
        candidate(segment, "document.Netto", "2.900,00 EUR", "August 2026"),
        candidate(
            segment,
            "document.Auszahlungsbetrag",
            "2.800,00 EUR",
            "August 2026",
        ),
        candidate(segment, "document.Personalnummer", "0004711", ""),
        candidate(segment, "document.IBAN", "DE89 3704 0044 0532 0130 00", ""),
        candidate(segment, "document.Bank", "Musterbank", ""),
        candidate(segment, "document.Abzug", "-100,00 EUR", "August 2026"),
        candidate(segment, "document.Eintritt", "01.03.2020", ""),
    ];
    facts.push(facts[0].clone());
    let checked = ground_extraction(&input, ExtractionOutput { facts });
    assert!(checked.rejected.is_empty());
    assert_eq!(checked.output.facts.len(), 9);
    assert_eq!(v.finish_extraction(&input, checked.output).unwrap(), 9);
    let proposals = v.proposals(item).unwrap();
    assert!(proposals.iter().any(|p| p.label.contains("August 2026")));
    assert!(proposals.iter().any(|p| p.label.contains("September 2026")));
    v.review_proposals(item, true).unwrap();
    assert!(!v.collection("0004711", false).unwrap().items.is_empty());
    drop(v);
    let v = Vault::unlock(&root, "synthetic-passphrase").unwrap();
    let items = v.collection("", false).unwrap();
    assert_eq!(
        items
            .items
            .iter()
            .filter(|i| i.title.starts_with("Gesamtbrutto ·"))
            .count(),
        2,
        "Equal pay in different months must stay distinct"
    );
    assert!(!v.collection("-100,00", false).unwrap().items.is_empty());
}

#[test]
fn document_evidence_rejects_fabricated_context_and_questions_remain_answerable() {
    let t = tempfile::tempdir().unwrap();
    let file = t.path().join("payroll.txt");
    std::fs::write(
        &file,
        "Erika Beispiel\nAugust 2026\nPersonalnummer: 0004711",
    )
    .unwrap();
    let mut v = Vault::create(&t.path().join("vault"), "synthetic-passphrase").unwrap();
    let item = v
        .import_document(&file, "Synthetic", DocumentClass::Personal)
        .unwrap();
    let input = v.prepare_extraction(item, "synthetic-model").unwrap();
    let mut fact = candidate(
        &input.segments[0].segment_id,
        "document.Personalnummer",
        "0004711",
        "September 2026",
    );
    assert!(
        v.finish_extraction(
            &input,
            ExtractionOutput {
                facts: vec![fact.clone()]
            }
        )
        .is_err()
    );
    let checked = ground_extraction(
        &input,
        ExtractionOutput {
            facts: vec![fact.clone()],
        },
    );
    assert_eq!(checked.rejected[0].code, "context_not_in_source");
    fact.context_quote.clear();
    fact.value = "0004712".into();
    let checked = ground_extraction(&input, ExtractionOutput { facts: vec![fact] });
    assert_eq!(checked.rejected.len(), 1);
    v.finish_extraction_with_questions(&input, checked.output, checked.rejected)
        .unwrap();
    let questions = v.review_questions(item).unwrap();
    assert_eq!(questions.len(), 1);
    assert!(questions[0].label.contains("Personalnummer"));
    v.answer_review_question(item, &questions[0].id, Some("0004711"))
        .unwrap();
    assert!(!v.collection("0004711", false).unwrap().items.is_empty());
}

#[test]
fn context_free_values_on_different_pages_are_not_collapsed() {
    let mut input = me_core::ExtractionInput {
        run_id: "r".into(),
        source_id: "s".into(),
        title: "bundle".into(),
        segments: vec![],
    };
    for (i, text) in [
        ("a", "Erika Beispiel 10,00 EUR"),
        ("b", "Erika Beispiel 10,00 EUR"),
    ] {
        input.segments.push(SourceText {
            segment_id: i.into(),
            ordinal: input.segments.len() as i64 + 1,
            text: text.into(),
        });
    }
    let checked = ground_extraction(
        &input,
        ExtractionOutput {
            facts: vec![
                candidate("a", "document.Betrag", "10,00 EUR", ""),
                candidate("b", "document.Betrag", "10,00 EUR", ""),
            ],
        },
    );
    assert_eq!(checked.output.facts.len(), 2);
}

#[test]
fn partial_numbers_and_lost_negative_signs_are_not_grounded() {
    let input = me_core::ExtractionInput {
        run_id: "r".into(),
        source_id: "s".into(),
        title: "t".into(),
        segments: vec![SourceText {
            segment_id: "a".into(),
            ordinal: 1,
            text: "Erika Beispiel\nBetrag: -100,00 EUR\nPersonalnummer: 0004711".into(),
        }],
    };
    for value in ["100,00 EUR", "00,00 EUR", "4711"] {
        let checked = ground_extraction(
            &input,
            ExtractionOutput {
                facts: vec![candidate("a", "document.Betrag", value, "")],
            },
        );
        assert!(checked.output.facts.is_empty());
    }
}
