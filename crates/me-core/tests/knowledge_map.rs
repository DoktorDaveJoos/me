use me_core::{
    DocumentClass, ExtractedFact, ExtractionInput, ExtractionOutput, HexPosition, KnowledgeKind,
    KnowledgeMap, KnowledgeRelation, KnowledgeStatus, KnowledgeViewport, RejectedFact, Vault,
};
use std::collections::{BTreeMap, BTreeSet};
const PASSWORD: &str = "synthetic-honeycomb-passphrase";
fn positions(graph: &KnowledgeMap) -> BTreeMap<String, HexPosition> {
    graph
        .nodes
        .iter()
        .map(|n| (n.id.clone(), n.position))
        .collect()
}
fn document(v: &mut Vault, dir: &std::path::Path, name: &str) -> (u64, ExtractionInput) {
    let path = dir.join(name);
    std::fs::write(&path,"SYNTHETIC Alex Morgan\nTax ID 01234567890\nEmployer Northstar Studio\nRole Product designer").unwrap();
    let item = v
        .import_document(&path, name, DocumentClass::Personal)
        .unwrap();
    v.begin_evaluation(item, false).unwrap();
    let input = v.prepare_extraction(item, "synthetic").unwrap();
    (item, input)
}
fn fact(input: &ExtractionInput, property: &str, value: &str) -> ExtractedFact {
    ExtractedFact {
        property: property.into(),
        value: value.into(),
        quote: value.into(),
        subject_quote: "Alex Morgan".into(),
        context_quote: String::new(),
        segment_id: input.segments[0].segment_id.clone(),
    }
}
#[test]
fn empty_map_has_no_placeholder_personal_information() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = Vault::create(&dir.path().join("vault"), PASSWORD).unwrap();
    let g = v.knowledge_map().unwrap();
    assert!(g.nodes.is_empty());
    assert!(g.edges.is_empty());
    assert!(g.groups.is_empty());
    assert_eq!(g.added, 0);
}
#[test]
fn positions_survive_refresh_edit_unlock_and_encrypted_backup() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("vault");
    let mut v = Vault::create(&root, PASSWORD).unwrap();
    let first = v
        .save_note(None, "Employer", "SYNTHETIC Northstar")
        .unwrap();
    v.save_note(None, "Role", "Designer").unwrap();
    let a = v.knowledge_map().unwrap();
    let before = positions(&a);
    assert_eq!(v.knowledge_map().unwrap().added, 0);
    assert_eq!(before, positions(&v.knowledge_map().unwrap()));
    v.save_note(Some(first), "Employer", "SYNTHETIC New employer")
        .unwrap();
    let edited = v.knowledge_map().unwrap();
    assert_eq!(before, positions(&edited));
    assert!(
        edited
            .nodes
            .iter()
            .any(|n| n.value == "SYNTHETIC New employer")
    );
    assert!(
        !edited
            .nodes
            .iter()
            .any(|n| n.value == "SYNTHETIC Northstar")
    );
    v.save_note(None, "Working hours", "40 hours").unwrap();
    let added = v.knowledge_map().unwrap();
    assert_eq!(added.added, 1);
    for (id, p) in &before {
        assert_eq!(&positions(&added)[id], p);
    }
    let view = KnowledgeViewport {
        center_q: 3.25,
        center_r: -8.75,
        zoom: 1.4,
        selected_node: Some(added.nodes[0].id.clone()),
    };
    v.save_knowledge_viewport(&view).unwrap();
    let saved = positions(&added);
    drop(v);
    let mut v = Vault::unlock(&root, PASSWORD).unwrap();
    assert_eq!(saved, positions(&v.knowledge_map().unwrap()));
    assert_eq!(view, v.knowledge_viewport().unwrap());
    let backup = dir.path().join("backup");
    v.backup(&backup).unwrap();
    let mut restored = Vault::restore(&backup, &dir.path().join("restored"), PASSWORD).unwrap();
    assert_eq!(saved, positions(&restored.knowledge_map().unwrap()));
    assert_eq!(view, restored.knowledge_viewport().unwrap());
    let encrypted = std::fs::read(root.join("vault.db")).unwrap();
    assert!(
        !encrypted
            .windows(b"SYNTHETIC New employer".len())
            .any(|w| w == b"SYNTHETIC New employer")
    );
}
#[test]
fn verified_proposals_keep_their_cells_and_all_sources_are_retained() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = Vault::create(&dir.path().join("vault"), PASSWORD).unwrap();
    v.set_automatic_evaluation(false).unwrap();
    let (item, input) = document(&mut v, dir.path(), "employment.txt");
    v.finish_extraction(
        &input,
        ExtractionOutput {
            facts: vec![
                fact(&input, "person.tax_id", "01234567890"),
                fact(&input, "document.Employer", "Northstar Studio"),
            ],
        },
    )
    .unwrap();
    v.finish_evaluation(item, None).unwrap();
    let pending = v.knowledge_map().unwrap();
    let cells = pending
        .nodes
        .iter()
        .filter(|n| n.kind == KnowledgeKind::Suggestion)
        .map(|n| (n.value.clone(), n.position))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(cells.len(), 2);
    let selected = pending
        .nodes
        .iter()
        .find(|n| n.kind == KnowledgeKind::Suggestion)
        .unwrap()
        .id
        .clone();
    v.save_knowledge_viewport(&KnowledgeViewport {
        selected_node: Some(selected),
        ..Default::default()
    })
    .unwrap();
    v.review_proposals(item, true).unwrap();
    let confirmed = v.knowledge_map().unwrap();
    for (value, p) in cells {
        let n = confirmed.nodes.iter().find(|n| n.value == value).unwrap();
        assert_eq!(n.position, p);
        assert_eq!(n.status, KnowledgeStatus::Confirmed);
        assert!(confirmed.edges.iter().any(|e| e.from == n.id
            && e.relation == KnowledgeRelation::Evidence
            && !e.quote.is_empty()));
    }
    assert!(
        confirmed
            .viewport
            .selected_node
            .as_ref()
            .is_some_and(|id| confirmed.node(id).is_some())
    );
    let (second, input) = document(&mut v, dir.path(), "second-source.txt");
    v.finish_extraction(
        &input,
        ExtractionOutput {
            facts: vec![fact(&input, "person.tax_id", "01234567890")],
        },
    )
    .unwrap();
    v.finish_evaluation(second, None).unwrap();
    v.review_proposals(second, true).unwrap();
    let graph = v.knowledge_map().unwrap();
    let tax = graph
        .nodes
        .iter()
        .find(|n| n.value == "01234567890")
        .unwrap();
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|n| n.value == "01234567890")
            .count(),
        1
    );
    assert_eq!(
        graph
            .edges
            .iter()
            .filter(|e| e.from == tax.id && e.relation == KnowledgeRelation::Evidence)
            .count(),
        2
    );
}
#[test]
fn unverified_suggestions_never_become_document_evidence_after_confirmation() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = Vault::create(&dir.path().join("vault"), PASSWORD).unwrap();
    v.set_automatic_evaluation(false).unwrap();
    let (item, input) = document(&mut v, dir.path(), "uncertain.txt");
    let mut proposed = fact(&input, "person.tax_id", "01234567890");
    proposed.quote = "SYNTHETIC invented quotation".into();
    v.finish_extraction_with_questions(
        &input,
        ExtractionOutput { facts: vec![] },
        vec![RejectedFact {
            fact: proposed,
            code: "quote_not_in_segment",
        }],
    )
    .unwrap();
    v.finish_evaluation(item, None).unwrap();
    let before = v.knowledge_map().unwrap();
    let question = before
        .nodes
        .iter()
        .find(|n| n.kind == KnowledgeKind::Suggestion)
        .unwrap();
    let position = question.position;
    assert_eq!(question.status, KnowledgeStatus::Unverified);
    assert!(
        before
            .edges
            .iter()
            .all(|e| e.quote.is_empty() && e.relation == KnowledgeRelation::Context)
    );
    let id = v.review_questions(item).unwrap()[0].id.clone();
    v.answer_review_question(item, &id, Some("01234567890"))
        .unwrap();
    let after = v.knowledge_map().unwrap();
    let n = after
        .nodes
        .iter()
        .find(|n| n.value == "01234567890")
        .unwrap();
    assert_eq!(n.status, KnowledgeStatus::Confirmed);
    assert_eq!(n.position, position);
    assert!(
        after
            .edges
            .iter()
            .all(|e| e.quote.is_empty() && e.relation == KnowledgeRelation::Context)
    );
}
#[test]
fn rejected_candidates_disappear_without_repacking_their_neighbors() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = Vault::create(&dir.path().join("vault"), PASSWORD).unwrap();
    let (item, input) = document(&mut v, dir.path(), "candidate.txt");
    v.finish_extraction(
        &input,
        ExtractionOutput {
            facts: vec![fact(&input, "person.tax_id", "01234567890")],
        },
    )
    .unwrap();
    v.finish_evaluation(item, None).unwrap();
    let before = v.knowledge_map().unwrap();
    v.review_proposals(item, false).unwrap();
    let after = v.knowledge_map().unwrap();
    assert_eq!(after.nodes.len(), 1);
    assert_eq!(
        after.nodes[0].position,
        before.node(&after.nodes[0].id).unwrap().position
    );
    assert!(after.edges.is_empty());
}
#[test]
fn conflicts_remain_explicit_and_restricted_documents_expose_only_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = Vault::create(&dir.path().join("vault"), PASSWORD).unwrap();
    v.save_note(None, "Tax ID", "01234567890").unwrap();
    v.save_note(None, "Tax ID", "11234567890").unwrap();
    let path = dir.path().join("restricted.txt");
    std::fs::write(&path, "SYNTHETIC PRIVATE CONTENT").unwrap();
    v.import_document(&path, "restricted", DocumentClass::Credential)
        .unwrap();
    let g = v.knowledge_map().unwrap();
    assert_eq!(
        g.nodes
            .iter()
            .filter(|n| n.status == KnowledgeStatus::Conflicting)
            .count(),
        2
    );
    let doc = g
        .nodes
        .iter()
        .find(|n| n.kind == KnowledgeKind::Document)
        .unwrap();
    assert_eq!(doc.status, KnowledgeStatus::Restricted);
    assert!(doc.value.is_empty());
    assert!(!format!("{g:?}").contains("SYNTHETIC PRIVATE CONTENT"));
}
#[test]
fn saved_camera_rejects_invalid_numbers_and_unknown_selection() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = Vault::create(&dir.path().join("vault"), PASSWORD).unwrap();
    for value in [f64::NAN, f64::INFINITY, -1., 3.] {
        assert!(
            v.save_knowledge_viewport(&KnowledgeViewport {
                zoom: value,
                ..Default::default()
            })
            .is_err()
        );
    }
    assert!(
        v.save_knowledge_viewport(&KnowledgeViewport {
            center_q: f64::NAN,
            ..Default::default()
        })
        .is_err()
    );
    v.save_knowledge_viewport(&KnowledgeViewport {
        selected_node: Some("unknown".into()),
        ..Default::default()
    })
    .unwrap();
    assert!(v.knowledge_viewport().unwrap().selected_node.is_none());
}
#[test]
fn dense_insertions_keep_unique_positions_and_do_not_move_existing_items() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = Vault::create(&dir.path().join("vault"), PASSWORD).unwrap();
    for i in 0..48 {
        v.save_note(None, &format!("Synthetic detail {i}"), "Example value")
            .unwrap();
    }
    let a = v.knowledge_map().unwrap();
    let before = positions(&a);
    for i in 48..64 {
        v.save_note(None, &format!("Synthetic detail {i}"), "Example value")
            .unwrap();
    }
    let b = v.knowledge_map().unwrap();
    assert_eq!(b.nodes.len(), 64);
    assert_eq!(
        b.nodes
            .iter()
            .map(|n| n.position)
            .collect::<BTreeSet<_>>()
            .len(),
        64
    );
    for (id, p) in before {
        assert_eq!(positions(&b)[&id], p);
    }
}
