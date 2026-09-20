use me_core::{
    DocumentClass, ExtractedFact, ExtractionInput, ExtractionOutput, RejectedFact, Vault,
};
const PASSWORD: &str = "synthetic-review-passphrase";
fn setup() -> (tempfile::TempDir, Vault, u64, ExtractionInput) {
    let temp = tempfile::tempdir().unwrap();
    let mut vault = Vault::create(&temp.path().join("vault"), PASSWORD).unwrap();
    let file = temp.path().join("test.txt");
    std::fs::write(
        &file,
        "Erika Beispiel\nSteuer-ID: 01234567890\nGeburtsdatum: 01.02.1990",
    )
    .unwrap();
    let item = vault
        .import_document(&file, "synthetic", DocumentClass::Personal)
        .unwrap();
    vault.set_automatic_evaluation(false).unwrap();
    vault.begin_evaluation(item, false).unwrap();
    let input = vault.prepare_extraction(item, "synthetic").unwrap();
    (temp, vault, item, input)
}
fn fact(input: &ExtractionInput) -> ExtractedFact {
    ExtractedFact {
        property: "person.tax_id".into(),
        value: "01234567890".into(),
        quote: "01234567890".into(),
        subject_quote: "Erika Beispiel".into(),
        context_quote: String::new(),
        segment_id: input.segments[0].segment_id.clone(),
    }
}
fn bad(input: &ExtractionInput, value: &str) -> RejectedFact {
    let mut fact = fact(input);
    fact.value = value.into();
    fact.quote = "SYNTHETIC invented quotation".into();
    RejectedFact {
        fact,
        code: "quote_not_in_segment",
    }
}
fn facts(vault: &Vault) -> serde_json::Value {
    vault
        .facts_get(&vault.shareable_scope().unwrap(), &["person.tax_id".into()])
        .unwrap()["facts"][0]
        .clone()
}

#[test]
fn ownerless_tax_id_is_asked_once_and_only_becomes_personal_after_confirmation() {
    let (_temp, mut vault, item, input) = setup();
    let mut candidate = fact(&input);
    candidate.subject_quote.clear();
    let grounded = me_core::ground_extraction(
        &input,
        ExtractionOutput {
            facts: vec![candidate],
        },
    );
    assert!(grounded.output.facts.is_empty());
    vault
        .finish_extraction_with_questions(&input, grounded.output, grounded.rejected)
        .unwrap();
    vault.finish_evaluation(item, None).unwrap();
    let question = vault.review_questions(item).unwrap().remove(0);
    assert!(question.reason.starts_with("Is this your tax ID?"));
    assert!(question.claimed_subject.is_empty());
    assert!(
        question
            .source_excerpt
            .as_ref()
            .unwrap()
            .contains("01234567890")
    );
    assert!(vault.proposals(item).unwrap().is_empty());
    assert_eq!(facts(&vault)["status"], "missing");
    vault.begin_evaluation(item, false).unwrap();
    let retry = vault.prepare_extraction(item, "synthetic").unwrap();
    let mut candidate = fact(&retry);
    candidate.subject_quote.clear();
    let grounded = me_core::ground_extraction(
        &retry,
        ExtractionOutput {
            facts: vec![candidate],
        },
    );
    vault
        .finish_extraction_with_questions(&retry, grounded.output, grounded.rejected)
        .unwrap();
    vault.finish_evaluation(item, None).unwrap();
    let questions = vault.review_questions(item).unwrap();
    assert_eq!(questions.len(), 1);
    assert_eq!(questions[0].id, question.id);
    vault
        .answer_review_question(item, &question.id, Some(&question.value))
        .unwrap();
    assert!(vault.review_questions(item).unwrap().is_empty());
    assert_eq!(facts(&vault)["value"], "01234567890");
    assert_eq!(
        facts(&vault)["candidates"][0]["locator"]["kind"],
        "manual_confirmation"
    );
}

#[test]
fn unanswered_suggestions_persist_encrypted_and_only_explicit_answers_become_manual_facts() {
    let (temp, mut vault, item, input) = setup();
    let mut unknown = bad(&input, "invalid-value");
    unknown.fact.segment_id = "other-source-segment".into();
    unknown.code = "unknown_segment";
    vault
        .finish_extraction_with_questions(
            &input,
            ExtractionOutput { facts: vec![] },
            vec![bad(&input, "01234567890"), unknown],
        )
        .unwrap();
    vault.finish_evaluation(item, None).unwrap();
    assert_eq!(facts(&vault)["status"], "missing");
    assert!(vault.proposals(item).unwrap().is_empty());
    let before = vault.review_questions(item).unwrap();
    assert_eq!(before.len(), 2);
    assert!(
        before[0]
            .source_excerpt
            .as_ref()
            .unwrap()
            .contains("Erika Beispiel")
    );
    assert!(before[1].source_excerpt.is_none());
    assert!(
        vault
            .evaluation_warning(item)
            .unwrap()
            .unwrap()
            .contains("2 details")
    );
    drop(vault);
    let bytes = std::fs::read(temp.path().join("vault/vault.db")).unwrap();
    assert!(
        !bytes
            .windows(b"SYNTHETIC invented quotation".len())
            .any(|w| w == b"SYNTHETIC invented quotation")
    );
    let mut vault = Vault::unlock(&temp.path().join("vault"), PASSWORD).unwrap();
    let questions = vault.review_questions(item).unwrap();
    assert_eq!(questions[0].id, before[0].id);
    assert!(
        matches!(&vault.collection("",false).unwrap().get(item).unwrap().content,me_core::Content::Document{state,..} if state=="needs_answer")
    );
    assert!(!vault.settings().unwrap().automatic_evaluation);
    assert!(
        vault
            .answer_review_question(item + 1, &questions[0].id, Some("01234567890"))
            .is_err()
    );
    assert!(
        vault
            .answer_review_question(item, &questions[1].id, Some("invalid-value"))
            .is_err()
    );
    assert_eq!(vault.review_questions(item).unwrap().len(), 2);
    vault
        .answer_review_question(item, &questions[0].id, Some("11234 567890"))
        .unwrap();
    let result = facts(&vault);
    assert_eq!(result["value"], "11234567890");
    let locator = &result["candidates"][0]["locator"];
    assert_eq!(locator["kind"], "manual_confirmation");
    assert!(locator["quote"].is_null());
    assert_ne!(result["candidates"][0]["source_id"], input.source_id);
    assert!(
        vault
            .answer_review_question(item, &questions[0].id, Some("01234567890"))
            .is_err()
    );
    assert_eq!(vault.review_questions(item).unwrap().len(), 1);
    vault
        .answer_review_question(item, &questions[1].id, None)
        .unwrap();
    assert!(vault.review_questions(item).unwrap().is_empty());
    assert!(vault.evaluation_warning(item).unwrap().is_none());
    assert_eq!(vault.collection("", false).unwrap().total, 2);
}

#[test]
fn retries_preserve_decisions_deduplicate_questions_and_promote_newly_supported_candidates() {
    let (_temp, mut vault, item, input) = setup();
    let mut good = fact(&input);
    good.property = "person.birth_date".into();
    good.value = "01.02.1990".into();
    good.quote = good.value.clone();
    vault
        .finish_extraction_with_questions(
            &input,
            ExtractionOutput { facts: vec![good] },
            vec![
                bad(&input, "01234567890"),
                bad(&input, "11234567890"),
                bad(&input, "11234567890"),
            ],
        )
        .unwrap();
    vault.finish_evaluation(item, None).unwrap();
    let questions = vault.review_questions(item).unwrap();
    assert_eq!(questions.len(), 2);
    vault.review_proposals(item, true).unwrap();
    assert_eq!(vault.review_questions(item).unwrap().len(), 2); // bulk approval never answers questions
    vault
        .answer_review_question(item, &questions[1].id, None)
        .unwrap();
    vault.begin_evaluation(item, false).unwrap();
    let retry = vault.prepare_extraction(item, "synthetic").unwrap();
    assert!(
        vault
            .answer_review_question(item, &questions[0].id, None)
            .is_err()
    );
    vault
        .finish_extraction_with_questions(
            &retry,
            ExtractionOutput { facts: vec![] },
            vec![bad(&retry, "01234567890"), bad(&retry, "11234567890")],
        )
        .unwrap();
    vault.finish_evaluation(item, None).unwrap();
    let open = vault.review_questions(item).unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].id, questions[0].id);
    vault.begin_evaluation(item, false).unwrap();
    let retry = vault.prepare_extraction(item, "synthetic").unwrap();
    vault
        .finish_extraction_with_questions(
            &retry,
            ExtractionOutput {
                facts: vec![fact(&retry)],
            },
            vec![],
        )
        .unwrap();
    vault.finish_evaluation(item, None).unwrap();
    assert!(vault.review_questions(item).unwrap().is_empty());
    assert!(vault.evaluation_warning(item).unwrap().is_none());
    assert_eq!(vault.proposals(item).unwrap().len(), 1);
    assert_eq!(facts(&vault)["status"], "unverified");
}

#[test]
fn invalid_completion_is_atomic_and_stale_runs_cannot_create_questions() {
    let (_temp, mut vault, item, input) = setup();
    let bad_output = ExtractionOutput {
        facts: vec![bad(&input, "01234567890").fact],
    };
    assert!(
        vault
            .finish_extraction_with_questions(&input, bad_output, vec![bad(&input, "11234567890")])
            .is_err()
    );
    assert!(vault.review_questions(item).unwrap().is_empty());
    vault.fail_extraction(&input.run_id).unwrap();
    assert!(
        vault
            .finish_extraction_with_questions(
                &input,
                ExtractionOutput { facts: vec![] },
                vec![bad(&input, "11234567890")]
            )
            .is_err()
    );
    assert!(vault.review_questions(item).unwrap().is_empty());
}

#[test]
fn confirmation_preserves_existing_conflicts_instead_of_overwriting_them() {
    let (_temp, mut vault, item, input) = setup();
    vault.save_note(None, "Steuer-ID", "01234567890").unwrap();
    vault
        .finish_extraction_with_questions(
            &input,
            ExtractionOutput { facts: vec![] },
            vec![bad(&input, "11234567890")],
        )
        .unwrap();
    vault.finish_evaluation(item, None).unwrap();
    let question = vault.review_questions(item).unwrap().remove(0);
    assert_eq!(question.existing_values, vec!["01234567890"]);
    vault
        .answer_review_question(item, &question.id, Some("11234567890"))
        .unwrap();
    assert_eq!(facts(&vault)["status"], "conflicting");
}

#[test]
fn selected_batch_is_atomic_and_keeps_unselected_questions_pending() {
    let (_temp, mut vault, item, input) = setup();
    vault
        .finish_extraction_with_questions(
            &input,
            ExtractionOutput { facts: vec![] },
            vec![bad(&input, "01234567890"), bad(&input, "11234567890")],
        )
        .unwrap();
    vault.finish_evaluation(item, None).unwrap();
    let entries = vault.pending_reviews().unwrap();
    assert_eq!(entries.len(), 2);
    assert!(
        vault
            .review_selected(&[entries[0].id.clone(), "stale-selection".into()], true)
            .is_err()
    );
    assert_eq!(vault.pending_reviews().unwrap().len(), 2);
    assert_eq!(facts(&vault)["status"], "missing");
    vault
        .review_selected(&[entries[0].id.clone()], true)
        .unwrap();
    assert_eq!(vault.pending_reviews().unwrap().len(), 1);
    assert_eq!(facts(&vault)["value"], "01234567890");
    assert_eq!(
        facts(&vault)["candidates"][0]["locator"]["kind"],
        "manual_confirmation"
    );
    vault
        .review_selected(&[entries[1].id.clone()], false)
        .unwrap();
    assert!(vault.pending_reviews().unwrap().is_empty());
    assert_eq!(facts(&vault)["value"], "01234567890");
}

#[test]
fn selected_batch_accepts_supported_and_uncertain_details_together() {
    let (_temp, mut vault, item, input) = setup();
    let mut birthday = fact(&input);
    birthday.property = "person.birth_date".into();
    birthday.value = "01.02.1990".into();
    birthday.quote = birthday.value.clone();
    vault
        .finish_extraction_with_questions(
            &input,
            ExtractionOutput {
                facts: vec![birthday],
            },
            vec![bad(&input, "01234567890")],
        )
        .unwrap();
    vault.finish_evaluation(item, None).unwrap();
    let ids = vault
        .pending_reviews()
        .unwrap()
        .iter()
        .map(|r| r.id.clone())
        .collect::<Vec<_>>();
    vault.review_selected(&ids, true).unwrap();
    assert!(vault.pending_reviews().unwrap().is_empty());
    assert_eq!(facts(&vault)["value"], "01234567890");
    assert_eq!(
        vault
            .facts_get(
                &vault.shareable_scope().unwrap(),
                &["person.birth_date".into()]
            )
            .unwrap()["facts"][0]["value"],
        "1990-02-01"
    );
}

#[test]
fn virtual_folders_persist_and_reject_unscoped_or_invalid_paths() {
    let (temp, mut vault, item, _input) = setup();
    let a = me_core::FolderAssignment {
        item,
        path: vec!["Personal".into(), "Tax".into()],
    };
    vault
        .save_document_folders(std::slice::from_ref(&a), &[item])
        .unwrap();
    let invalid = me_core::FolderAssignment {
        item,
        path: vec!["../private".into()],
    };
    assert!(vault.save_document_folders(&[invalid], &[item]).is_err());
    assert!(vault.save_document_folders(&[a], &[]).is_err());
    assert_eq!(vault.document_folders().unwrap()[&item], "Personal/Tax");
    drop(vault);
    let vault = Vault::unlock(&temp.path().join("vault"), PASSWORD).unwrap();
    assert_eq!(vault.document_folders().unwrap()[&item], "Personal/Tax");
    assert_eq!(
        vault
            .collection("", false)
            .unwrap()
            .get(item)
            .unwrap()
            .title,
        "test.txt"
    );
}

#[test]
fn english_queries_retrieve_german_fields_without_releasing_credentials() {
    let (temp, mut vault, _item, _input) = setup();
    vault
        .save_note(None, "Social insurance number", "12010290B123")
        .unwrap();
    let secret = temp.path().join("secret.txt");
    std::fs::write(&secret, "SYNTHETIC_CREDENTIAL_VALUE").unwrap();
    let secret_id = vault
        .import_document(&secret, "secret", DocumentClass::Credential)
        .unwrap();
    let context = vault
        .chat_context(
            &["Steuer".into()],
            &["person.social_insurance_number".into()],
            &[secret_id],
        )
        .unwrap();
    assert_eq!(context["facts"][0]["value"], "12010290B123");
    assert!(!context.to_string().contains("SYNTHETIC_CREDENTIAL_VALUE"));
    assert!(
        !vault
            .organization_input()
            .unwrap()
            .to_string()
            .contains("SYNTHETIC_CREDENTIAL_VALUE")
    );
    for i in 0..205 {
        vault
            .save_note(None, &format!("Synthetic note {i}"), "example")
            .unwrap();
    }
    assert!(
        vault
            .collection("", false)
            .unwrap()
            .items
            .iter()
            .any(|i| i.id == secret_id)
    );
}

#[test]
fn approved_document_details_are_explicit_in_chat_context() {
    let (_temp, mut vault, item, input) = setup();
    let mut bank = bad(&input, "DE02120300000000202051");
    bank.fact.property = "document.Bank account".into();
    vault
        .finish_extraction_with_questions(&input, ExtractionOutput { facts: vec![] }, vec![bank])
        .unwrap();
    vault.finish_evaluation(item, None).unwrap();
    let ids = vault
        .pending_reviews()
        .unwrap()
        .iter()
        .map(|r| r.id.clone())
        .collect::<Vec<_>>();
    vault.review_selected(&ids, true).unwrap();
    let context = vault.chat_context(&["Bank".into()], &[], &[]).unwrap();
    assert_eq!(
        context["confirmed_details"][0]["value"],
        "DE02120300000000202051"
    );
    assert_eq!(context["confirmed_details"][0]["state"], "accepted");
}
