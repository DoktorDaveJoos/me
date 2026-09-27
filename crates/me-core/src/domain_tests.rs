use crate::*;
use serde_json::{Value, json};
use zeroize::Zeroizing;
const PASSWORD: &str = "synthetic-domain-test-password";
fn setup() -> (tempfile::TempDir, Vault) {
    let t = tempfile::tempdir().unwrap();
    let v = Vault::create(&t.path().join("vault"), PASSWORD).unwrap();
    (t, v)
}
fn property(v: &mut Vault, key: &str, kind: &str, many: bool) {
    v.define_property(&PropertyDefinition {
        key: key.into(),
        label: key.into(),
        value_type: kind.into(),
        many,
        subject_kinds: Vec::new(),
        object_kinds: Vec::new(),
    })
    .unwrap();
}
fn money(subject: &str, amount: &str, from: &str, to: Option<&str>) -> AssertionDraft {
    AssertionDraft {
        subject: subject.into(),
        property: "contract.premium".into(),
        value: FactValue::Money {
            amount: amount.into(),
            currency: "EUR".into(),
            period: None,
        },
        validity: Validity::Interval {
            from: from.into(),
            to: to.map(str::to_owned),
        },
    }
}
fn update(revision: i64, password: &str) -> LoginUpdate {
    LoginUpdate {
        revision,
        title: "Synthetic insurance".into(),
        favorite: false,
        fields: vec![("password".into(), Zeroizing::new(password.into()))],
    }
}
fn last(v: &Vault) -> i64 {
    v.db.query_row(
        "SELECT coalesce(max(sequence),0) FROM domain_revision",
        [],
        |r| r.get(0),
    )
    .unwrap()
}
fn decrypt(v: &Vault, r: &EncryptedRevision) -> Value {
    let plain = crate::crypto::open(
        &v.keys[32..],
        &r.ciphertext,
        format!("me-domain-revision-v1:{}:{}", v.header.vault_id, r.id).as_bytes(),
    )
    .unwrap();
    serde_json::from_slice(&plain).unwrap()
}

#[test]
fn dates_types_and_cross_document_identity_are_not_flat_profile_strings() {
    let (_t, mut v) = setup();
    let contract = v
        .create_entity(EntityKind::InsuranceContract, "Synthetic contract")
        .unwrap();
    property(&mut v, "contract.premium", "money", false);
    v.record_fact(&money(
        &contract,
        "0700.00",
        "2026-01-01",
        Some("2027-01-01"),
    ))
    .unwrap();
    v.record_fact(&money(&contract, "742.00", "2027-01-01", None))
        .unwrap();
    let old = v
        .resolve_fact_at(&contract, "contract.premium", "2026-09-26")
        .unwrap();
    assert_eq!(old.status, ResolutionStatus::Resolved);
    assert_eq!(old.facts[0].value, json!({"amount":"700","currency":"EUR"}));
    let future = v
        .resolve_fact_at(&contract, "contract.premium", "2027-01-01")
        .unwrap();
    assert_eq!(future.facts[0].value["amount"], "742");
    assert!(
        v.resolve_fact_at(
            &v.profile_entity_id().unwrap(),
            "contract.premium",
            "2027-01-01"
        )
        .unwrap()
        .facts
        .is_empty()
    );
    v.record_fact(&money(&contract, "750", "2027-01-01", None))
        .unwrap();
    assert_eq!(
        v.resolve_fact_at(&contract, "contract.premium", "2027-03-01")
            .unwrap()
            .status,
        ResolutionStatus::Conflict
    );
    let before = last(&v);
    assert!(
        v.record_fact(&money(&contract, "1e3", "2026-01-01", None))
            .is_err()
    );
    assert!(
        v.record_fact(&money(&contract, "1.00", "2027-02-30", None))
            .is_err()
    );
    assert_eq!(last(&v), before);
    let incompatible = PropertyDefinition {
        key: "contract.premium".into(),
        label: "Oops".into(),
        value_type: "text".into(),
        many: true,
        subject_kinds: Vec::new(),
        object_kinds: Vec::new(),
    };
    assert!(v.define_property(&incompatible).is_err());
    assert!(
        v.collection("", false)
            .unwrap()
            .items
            .iter()
            .any(|i| matches!(&i.content,Content::Note(s) if s=="700 EUR"))
    );
}

#[test]
fn relationships_identity_matching_and_reversible_merges_preserve_original_assertions() {
    let (_t, mut v) = setup();
    let one = v
        .create_entity(EntityKind::Vehicle, "Same display name")
        .unwrap();
    let two = v
        .create_entity(EntityKind::Vehicle, "Same display name")
        .unwrap();
    let other = v
        .create_entity(EntityKind::Person, "Same display name")
        .unwrap();
    v.add_entity_identifier(&one, "vehicle.vin", "SYNTHETIC-VIN")
        .unwrap();
    v.add_entity_identifier(&two, "vehicle.vin", "SYNTHETIC-VIN")
        .unwrap();
    assert_eq!(
        v.matching_entities("vehicle.vin", "SYNTHETIC-VIN")
            .unwrap()
            .len(),
        2
    );
    property(&mut v, "vehicle.label", "text", false);
    let assertion = v
        .record_fact(&AssertionDraft {
            subject: one.clone(),
            property: "vehicle.label".into(),
            value: FactValue::Text("Supported label".into()),
            validity: Validity::Timeless,
        })
        .unwrap();
    assert!(v.merge_entities(&one, &other).is_err());
    let merge = v.merge_entities(&one, &two).unwrap();
    assert_eq!(
        v.matching_entities("vehicle.vin", "SYNTHETIC-VIN").unwrap(),
        vec![two.clone()]
    );
    assert_eq!(
        v.resolve_fact_at(&two, "vehicle.label", "2026-09-26")
            .unwrap()
            .facts[0]
            .id,
        assertion
    );
    assert!(v.merge_entities(&two, &one).is_err());
    assert!(v.delete_entity(&two).is_err());
    v.reverse_entity_merge(&merge).unwrap();
    assert!(
        v.resolve_fact_at(&two, "vehicle.label", "2026-09-26")
            .unwrap()
            .facts
            .is_empty()
    );
    property(&mut v, "person.owns", "entity", true);
    v.record_fact(&AssertionDraft {
        subject: v.profile_entity_id().unwrap(),
        property: "person.owns".into(),
        value: FactValue::Entity(one.clone()),
        validity: Validity::Timeless,
    })
    .unwrap();
    assert_eq!(
        v.db.query_row("SELECT count(*) FROM accepted_edges", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    v.delete_entity(&one).unwrap();
    assert!(
        v.resolve_fact_at(&one, "vehicle.label", "2026-09-26")
            .is_err()
    );
    let body:String=v.db.query_row("SELECT payload_json FROM domain_revision WHERE record_kind='entity' AND record_id=? ORDER BY sequence DESC LIMIT 1",[one],|r|r.get(0)).unwrap();
    assert!(serde_json::from_str::<Value>(&body).unwrap()["deleted_at"].is_string());
}

#[test]
fn extracted_observations_keep_evidence_and_reviewed_mapping_does_not_fabricate_quotes() {
    let (t, mut v) = setup();
    let file = t.path().join("insurance.txt");
    std::fs::write(
        &file,
        "Synthetic Person premium 742 EUR effective 2027-01-01",
    )
    .unwrap();
    let item = v
        .import_document(&file, "Insurance", DocumentClass::Personal)
        .unwrap();
    v.enable_text_search(item).unwrap();
    let input = v.prepare_extraction(item, "synthetic").unwrap();
    v.finish_extraction(
        &input,
        ExtractionOutput {
            facts: vec![ExtractedFact {
                property: "document.Premium".into(),
                value: "742".into(),
                segment_id: input.segments[0].segment_id.clone(),
                quote: input.segments[0].text.clone(),
                subject_quote: "Synthetic Person".into(),
                context_quote: "2027-01-01".into(),
            }],
        },
    )
    .unwrap();
    let observations = v.observations(&input.source_id).unwrap();
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].context_quote, "2027-01-01");
    assert_eq!(observations[0].verification, "supported");
    let contract = v
        .create_entity(EntityKind::InsuranceContract, "Contract 1837")
        .unwrap();
    property(&mut v, "contract.premium", "money", false);
    let a = v
        .resolve_observation(
            &observations[0].id,
            &money(&contract, "742", "2027-01-01", None),
        )
        .unwrap();
    let (origin,source_kind,locator):(String,String,String)=v.db.query_row("SELECT a.origin,s.kind,e.locator_json FROM assertion a JOIN assertion_evidence e ON e.assertion_id=a.id JOIN source s ON s.id=e.source_id WHERE a.id=?",[a],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(origin, "user");
    assert_eq!(source_kind, "note");
    assert_eq!(
        serde_json::from_str::<Value>(&locator).unwrap()["kind"],
        "reviewed_mapping"
    );
    assert!(!locator.contains("quote"));
    assert_eq!(v.observations(&input.source_id).unwrap()[0].value, "742");
}

#[test]
fn legacy_mutations_have_stable_causal_encrypted_revisions_and_secret_versions() {
    let (_t, mut v) = setup();
    let note = v
        .save_note(None, "Synthetic note", "PRIVATE_SENTINEL_123")
        .unwrap();
    let item = v
        .collection("", false)
        .unwrap()
        .get(note)
        .unwrap()
        .stable_id
        .clone();
    let before = last(&v);
    v.save_note(Some(note), "Synthetic note", "PRIVATE_SENTINEL_456")
        .unwrap();
    let batch = v.encrypted_revisions(before, 100).unwrap();
    assert!(!batch.revisions.is_empty());
    assert!(batch.local_cursor > before);
    let record = batch
        .revisions
        .iter()
        .map(|r| decrypt(&v, r))
        .find(|r| r["record_kind"] == "collection_item")
        .unwrap();
    assert_eq!(record["record_id"], item);
    assert!(record["payload"].get("local_id").is_none());
    assert_eq!(record["parents"].as_array().unwrap().len(), 1);
    for r in &batch.revisions {
        assert!(!r.ciphertext.windows(16).any(|w| w == b"PRIVATE_SENTINEL_"));
        assert!(crate::crypto::open(&v.keys[32..], &r.ciphertext, b"wrong-context").is_err());
    }
    let credential = v
        .create_credential(CredentialKind::Login, update(0, "FIRST_SECRET"))
        .unwrap();
    let stable = v
        .collection("", false)
        .unwrap()
        .get(credential)
        .unwrap()
        .stable_id
        .clone();
    let first = v.credential_version_ids(&stable).unwrap();
    assert_eq!(first.len(), 1);
    v.update_login(credential, update(1, "SECOND_SECRET"))
        .unwrap();
    let versions = v.credential_version_ids(&stable).unwrap();
    assert_eq!(versions.len(), 2);
    let parent: String =
        v.db.query_row(
            "SELECT parent_id FROM secret_version WHERE id=?",
            [&versions[1]],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(parent, first[0]);
    v.delete_login(credential, 2).unwrap();
    let payload:String=v.db.query_row("SELECT payload_json FROM domain_revision WHERE record_kind='collection_item' AND record_id=? ORDER BY sequence DESC LIMIT 1",[&stable],|r|r.get(0)).unwrap();
    assert!(serde_json::from_str::<Value>(&payload).unwrap()["deleted_at"].is_string());
    v.restore_login(credential, 3).unwrap();
    assert_eq!(v.credential_version_ids(&stable).unwrap().len(), 2);
    v.verify().unwrap();
}

#[test]
fn restored_replicas_have_distinct_devices_and_preserve_concurrent_ancestry() {
    let (t, mut a) = setup();
    let note = a.save_note(None, "Shared", "Before").unwrap();
    let stable = a
        .collection("", false)
        .unwrap()
        .get(note)
        .unwrap()
        .stable_id
        .clone();
    let backup = t.path().join("backup");
    a.backup(&backup).unwrap();
    let mut b = Vault::restore(&backup, &t.path().join("replica"), PASSWORD).unwrap();
    assert_ne!(a.device_identity().unwrap(), b.device_identity().unwrap());
    a.save_note(Some(note), "Shared", "Edit A").unwrap();
    b.save_note(Some(note), "Shared", "Edit B").unwrap();
    let row = |v: &Vault| {
        v.db.query_row("SELECT id,parents_json,device_id FROM domain_revision WHERE record_kind='collection_item' AND record_id=? ORDER BY sequence DESC LIMIT 1",[&stable],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).unwrap()
    };
    let ar = row(&a);
    let br = row(&b);
    assert_ne!(ar.0, br.0);
    assert_eq!(ar.1, br.1);
    assert_ne!(ar.2, br.2);
    let original_device = b.device_identity().unwrap();
    drop(b);
    let b = Vault::unlock(&t.path().join("replica"), PASSWORD).unwrap();
    assert_eq!(b.device_identity().unwrap(), original_device);
}

#[test]
fn grants_enforce_task_field_destination_revocation_and_expiry_with_audit() {
    let (_t, mut v) = setup();
    let person = v.profile_entity_id().unwrap();
    property(&mut v, "person.address", "text", false);
    v.record_fact(&AssertionDraft {
        subject: person.clone(),
        property: "person.address".into(),
        value: FactValue::Text("SYNTHETIC_PRIVATE_ADDRESS".into()),
        validity: Validity::Timeless,
    })
    .unwrap();
    let principal = v.register_agent_principal("Synthetic agent").unwrap();
    let task = v
        .create_action_intent("Compare insurance", None, None, None)
        .unwrap();
    let resource = AccessResource::Fact {
        entity: person.clone(),
        property: "person.address".into(),
    };
    let grant = v
        .issue_access_grant(
            &principal,
            &task,
            &resource,
            AccessOperation::Read,
            "Compare insurance",
            600,
        )
        .unwrap();
    assert_eq!(
        v.read_granted_fact(&grant, &person, "person.address", "2026-09-26")
            .unwrap()
            .status,
        ResolutionStatus::Resolved
    );
    assert!(
        v.read_granted_fact(&grant, &person, "person.tax_id", "2026-09-26")
            .is_err()
    );
    v.revoke_access_grant(&grant).unwrap();
    assert!(
        v.read_granted_fact(&grant, &person, "person.address", "2026-09-26")
            .is_err()
    );
    let expired = v
        .issue_access_grant(
            &principal,
            &task,
            &resource,
            AccessOperation::Read,
            "Compare insurance",
            1,
        )
        .unwrap();
    v.db.execute(
        "UPDATE capability_grant SET issued_at=unixepoch()-2,expires_at=unixepoch()-1 WHERE id=?",
        [expired.id()],
    )
    .unwrap();
    assert!(
        v.read_granted_fact(&expired, &person, "person.address", "2026-09-26")
            .is_err()
    );
    let item = v
        .create_credential(CredentialKind::Login, update(0, "BROKER_ONLY_SECRET"))
        .unwrap();
    let credential = v
        .collection("", false)
        .unwrap()
        .get(item)
        .unwrap()
        .stable_id
        .clone();
    let resource = AccessResource::Credential {
        credential: credential.clone(),
        origin: "https://insurer.example".into(),
    };
    let use_grant = v
        .issue_access_grant(
            &principal,
            &task,
            &resource,
            AccessOperation::CredentialUse,
            "Compare insurance",
            600,
        )
        .unwrap();
    assert!(
        v.credential_for_broker(&use_grant, &credential, "https://evil.example")
            .is_err()
    );
    assert!(
        v.credential_for_broker(&use_grant, &credential, "https://insurer.example")
            .unwrap()
            .fields
            .iter()
            .any(|(_, s)| s.as_str() == "BROKER_ONLY_SECRET")
    );
    let reveal = v
        .issue_access_grant(
            &principal,
            &task,
            &resource,
            AccessOperation::SecretReveal,
            "Explicit reveal",
            600,
        )
        .unwrap();
    assert!(
        v.credential_for_broker(&reveal, &credential, "https://insurer.example")
            .is_err()
    );
    let audit: String =
        v.db.query_row(
            "SELECT group_concat(resource_id||outcome) FROM access_audit",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!audit.contains("BROKER_ONLY_SECRET"));
    assert!(!audit.contains("SYNTHETIC_PRIVATE_ADDRESS"));
    assert!(audit.contains("denied"));
}

#[test]
fn action_approval_is_exact_and_execution_is_not_blindly_repeated() {
    let (t, mut v) = setup();
    let task = v
        .create_action_intent("Prepare cancellation", None, None, None)
        .unwrap();
    let principal = v.register_agent_principal("Synthetic executor").unwrap();
    let grant = v
        .issue_access_grant(
            &principal,
            &task,
            &AccessResource::Action(task.clone()),
            AccessOperation::Execute,
            "User-approved cancellation",
            600,
        )
        .unwrap();
    let mut draft = ActionDraft {
        operation: "send_email".into(),
        recipient: "test@example.invalid".into(),
        body: "Synthetic draft A".into(),
        attachments: vec![],
    };
    let first = v.draft_action(&task, None, &draft).unwrap();
    let old_approval = v.approve_action(&first).unwrap();
    draft.body = "Synthetic draft B".into();
    let second = v.draft_action(&task, Some(&first), &draft).unwrap();
    assert!(
        v.begin_action_attempt(&grant, &first, &old_approval)
            .is_err()
    );
    assert!(
        v.begin_action_attempt(&grant, &second, &old_approval)
            .is_err()
    );
    let approval = v.approve_action(&second).unwrap();
    let attempt = v.begin_action_attempt(&grant, &second, &approval).unwrap();
    assert!(v.begin_action_attempt(&grant, &second, &approval).is_err());
    drop(v);
    let mut v = Vault::unlock(&t.path().join("vault"), PASSWORD).unwrap();
    let state: String =
        v.db.query_row(
            "SELECT state FROM action_attempt WHERE id=?",
            [&attempt],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "indeterminate");
    assert!(v.begin_action_attempt(&grant, &second, &approval).is_err());
    assert!(
        v.finish_action_attempt(&attempt, ActionOutcome::Succeeded)
            .is_err()
    );
}

#[test]
fn migration_from_thirteen_preserves_content_and_bootstraps_once() {
    let (t, mut v) = setup();
    let note = v.save_note(None, "Legacy", "Legacy value").unwrap();
    let credential = v
        .create_credential(CredentialKind::Login, update(0, "LEGACY_SECRET"))
        .unwrap();
    let stable = v
        .collection("", false)
        .unwrap()
        .get(credential)
        .unwrap()
        .stable_id
        .clone();
    crate::revisions::remove_for_legacy_fixture(&v.db);
    v.db.pragma_update(None, "user_version", 13).unwrap();
    drop(v);
    let v = Vault::unlock(&t.path().join("vault"), PASSWORD).unwrap();
    assert!(
        matches!(&v.collection("",false).unwrap().get(note).unwrap().content,Content::Note(s) if s=="Legacy value")
    );
    assert_eq!(v.credential_version_ids(&stable).unwrap().len(), 1);
    let count = last(&v);
    v.verify().unwrap();
    drop(v);
    let v = Vault::unlock(&t.path().join("vault"), PASSWORD).unwrap();
    assert_eq!(last(&v), count);
    assert_eq!(
        v.db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        16
    );
}

#[test]
fn failed_legacy_transaction_rolls_back_domain_history_too() {
    let (_t, mut v) = setup();
    let count = last(&v);
    v.db.execute_batch("CREATE TRIGGER synthetic_failure BEFORE INSERT ON collection_item BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(
        v.save_note(None, "Atomic", "Do not retain partial data")
            .is_err()
    );
    assert_eq!(last(&v), count);
    assert!(v.collection("", false).unwrap().is_empty());
}

#[test]
fn field_grant_does_not_reveal_restricted_provenance_or_hidden_conflict_status() {
    let (_t, mut v) = setup();
    let entity = v.profile_entity_id().unwrap();
    property(&mut v, "person.address", "text", false);
    let assertion = v
        .record_fact(&AssertionDraft {
            subject: entity.clone(),
            property: "person.address".into(),
            value: FactValue::Text("HIDDEN_VALUE".into()),
            validity: Validity::Timeless,
        })
        .unwrap();
    v.db.execute("UPDATE source SET sensitivity='restricted' WHERE id IN(SELECT source_id FROM assertion_evidence WHERE assertion_id=?)",[assertion]).unwrap();
    let principal = v.register_agent_principal("Synthetic agent").unwrap();
    let task = v
        .create_action_intent("Read selected address", None, None, None)
        .unwrap();
    let grant = v
        .issue_access_grant(
            &principal,
            &task,
            &AccessResource::Fact {
                entity: entity.clone(),
                property: "person.address".into(),
            },
            AccessOperation::Read,
            "Address only",
            600,
        )
        .unwrap();
    let result = v
        .read_granted_fact(&grant, &entity, "person.address", "2026-09-26")
        .unwrap();
    assert_eq!(result.status, ResolutionStatus::Missing);
    assert!(result.facts.is_empty());
}

#[test]
fn temporal_graph_keeps_history_without_false_conflicts_and_retry_ciphertext_is_stable() {
    let (_t, mut v) = setup();
    let contract = v
        .create_entity(EntityKind::InsuranceContract, "Synthetic policy")
        .unwrap();
    property(&mut v, "contract.premium", "money", false);
    v.record_fact(&money(&contract, "700", "2026-01-01", Some("2027-01-01")))
        .unwrap();
    v.record_fact(&money(&contract, "742", "2027-01-01", None))
        .unwrap();
    let graph = v.knowledge_map().unwrap();
    assert!(
        graph
            .nodes
            .iter()
            .all(|n| n.status != KnowledgeStatus::Conflicting)
    );
    assert!(
        graph
            .nodes
            .iter()
            .any(|n| n.context.contains("2027-01-01") && n.value == "742 EUR")
    );
    v.record_fact(&money(&contract, "750", "2027-01-01", None))
        .unwrap();
    let graph = v.knowledge_map().unwrap();
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|n| n.status == KnowledgeStatus::Conflicting)
            .count(),
        2
    );
    let first = v.encrypted_revisions(0, 100).unwrap();
    let second = v.encrypted_revisions(0, 100).unwrap();
    assert_eq!(first.local_cursor, second.local_cursor);
    assert_eq!(
        first
            .revisions
            .iter()
            .map(|r| &r.ciphertext)
            .collect::<Vec<_>>(),
        second
            .revisions
            .iter()
            .map(|r| &r.ciphertext)
            .collect::<Vec<_>>()
    );
}

#[test]
fn merging_entities_never_expands_an_existing_field_grant() {
    let (_t, mut v) = setup();
    let a = v.create_entity(EntityKind::Person, "A").unwrap();
    let b = v.create_entity(EntityKind::Person, "B").unwrap();
    property(&mut v, "person.address", "text", false);
    for (entity, value) in [(&a, "HIDDEN_A_ADDRESS"), (&b, "ALLOWED_B_ADDRESS")] {
        v.record_fact(&AssertionDraft {
            subject: entity.clone(),
            property: "person.address".into(),
            value: FactValue::Text(value.into()),
            validity: Validity::Timeless,
        })
        .unwrap();
    }
    let principal = v.register_agent_principal("Synthetic agent").unwrap();
    let task = v
        .create_action_intent("Read B address", None, None, None)
        .unwrap();
    let grant = v
        .issue_access_grant(
            &principal,
            &task,
            &AccessResource::Fact {
                entity: b.clone(),
                property: "person.address".into(),
            },
            AccessOperation::Read,
            "Only B",
            600,
        )
        .unwrap();
    v.merge_entities(&a, &b).unwrap();
    let result = v
        .read_granted_fact(&grant, &b, "person.address", "2026-09-26")
        .unwrap();
    assert_eq!(result.status, ResolutionStatus::Resolved);
    assert_eq!(result.facts.len(), 1);
    assert_eq!(result.facts[0].value, "ALLOWED_B_ADDRESS");
}

#[test]
fn source_and_action_grants_do_not_authorize_each_others_operations() {
    let (_t, mut v) = setup();
    let note = v.save_note(None, "Synthetic", "Source body").unwrap();
    let source: String =
        v.db.query_row(
            "SELECT source_id FROM collection_item WHERE local_id=?",
            [note as i64],
            |r| r.get(0),
        )
        .unwrap();
    let principal = v.register_agent_principal("Synthetic agent").unwrap();
    let task = v
        .create_action_intent("Draft only", None, None, None)
        .unwrap();
    let source_grant = v
        .issue_access_grant(
            &principal,
            &task,
            &AccessResource::Source(source.clone()),
            AccessOperation::Read,
            "Read selected source",
            600,
        )
        .unwrap();
    assert!(
        !v.read_granted_source(&source_grant, &source)
            .unwrap()
            .is_empty()
    );
    let draft = ActionDraft {
        operation: "email".into(),
        recipient: "test@example.invalid".into(),
        body: "Synthetic draft".into(),
        attachments: vec![],
    };
    assert!(
        v.draft_granted_action(&source_grant, &task, None, &draft)
            .is_err()
    );
    let grant = v
        .issue_access_grant(
            &principal,
            &task,
            &AccessResource::Action(task.clone()),
            AccessOperation::Draft,
            "Draft only",
            600,
        )
        .unwrap();
    let revision = v.draft_granted_action(&grant, &task, None, &draft).unwrap();
    let approval = v.approve_action(&revision).unwrap();
    assert!(
        v.begin_action_attempt(&grant, &revision, &approval)
            .is_err()
    );
    v.delete_action_intent(&task).unwrap();
    assert!(v.read_granted_source(&source_grant, &source).is_err());
}

#[test]
fn extraction_reruns_retain_distinct_observations_without_duplicate_canonical_mapping() {
    let (t, mut v) = setup();
    let file = t.path().join("premium.txt");
    std::fs::write(&file, "Synthetic Person premium 742 EUR").unwrap();
    let item = v
        .import_document(&file, "Premium", DocumentClass::Personal)
        .unwrap();
    v.enable_text_search(item).unwrap();
    let mut source = String::new();
    for _ in 0..2 {
        let input = v.prepare_extraction(item, "synthetic").unwrap();
        source = input.source_id.clone();
        v.finish_extraction(
            &input,
            ExtractionOutput {
                facts: vec![ExtractedFact {
                    property: "document.Premium".into(),
                    value: "742".into(),
                    quote: input.segments[0].text.clone(),
                    segment_id: input.segments[0].segment_id.clone(),
                    subject_quote: "Synthetic Person".into(),
                    context_quote: String::new(),
                }],
            },
        )
        .unwrap();
    }
    let observations = v.observations(&source).unwrap();
    assert_eq!(observations.len(), 2);
    let contract = v
        .create_entity(EntityKind::InsuranceContract, "Synthetic policy")
        .unwrap();
    property(&mut v, "contract.premium", "money", false);
    let draft = money(&contract, "742", "2027-01-01", None);
    let first = v.resolve_observation(&observations[0].id, &draft).unwrap();
    let count = v.collection("", false).unwrap().total;
    assert_eq!(
        v.resolve_observation(&observations[0].id, &draft).unwrap(),
        first
    );
    assert_eq!(v.collection("", false).unwrap().total, count);
}

#[test]
fn initial_revisions_use_the_persistent_device_and_asset_retries_are_identical() {
    let (t, mut v) = setup();
    let devices: Vec<String> =
        v.db.prepare("SELECT DISTINCT device_id FROM domain_revision")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
    assert_eq!(devices, vec![v.device_identity().unwrap()]);
    let file = t.path().join("asset.txt");
    std::fs::write(&file, "SYNTHETIC_ASSET").unwrap();
    let item = v
        .import_document(&file, "Asset", DocumentClass::Personal)
        .unwrap();
    let object:String=v.db.query_row("SELECT s.object_id FROM source s JOIN collection_item i ON i.source_id=s.id WHERE i.local_id=?",[item as i64],|r|r.get(0)).unwrap();
    let first = v.encrypted_replication_asset("document", &object).unwrap();
    assert_eq!(
        first,
        v.encrypted_replication_asset("document", &object).unwrap()
    );
    let context = format!("me-domain-asset-v1:{}:document:{object}", v.header.vault_id);
    assert_eq!(
        crate::crypto::open(&v.keys[32..], &first, context.as_bytes())
            .unwrap()
            .as_slice(),
        b"SYNTHETIC_ASSET"
    );
}

#[test]
fn typed_api_retains_existing_standard_field_validation() {
    let (_t, mut v) = setup();
    let profile = v.profile_entity_id().unwrap();
    let mut draft = AssertionDraft {
        subject: profile,
        property: "person.tax_id".into(),
        value: FactValue::Identifier("123".into()),
        validity: Validity::Timeless,
    };
    assert!(v.record_fact(&draft).is_err());
    draft.value = FactValue::Identifier("00 123 456 789".into());
    let a = v.record_fact(&draft).unwrap();
    let raw: String =
        v.db.query_row("SELECT value_json FROM assertion WHERE id=?", [a], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(serde_json::from_str::<String>(&raw).unwrap(), "00123456789");
}

fn at(from: &str) -> Validity {
    Validity::Interval {
        from: from.into(),
        to: None,
    }
}
fn fact(subject: &str, property: &str, value: FactValue) -> AssertionDraft {
    AssertionDraft {
        subject: subject.into(),
        property: property.into(),
        value,
        validity: at("2025-01-01"),
    }
}
fn link(subject: &str, property: &str, object: &str) -> AssertionDraft {
    fact(subject, property, FactValue::Entity(object.into()))
}
fn text(subject: &str, property: &str, value: &str) -> AssertionDraft {
    fact(subject, property, FactValue::Text(value.into()))
}
fn per_month(subject: &str, property: &str, amount: &str) -> AssertionDraft {
    fact(
        subject,
        property,
        FactValue::Money {
            amount: amount.into(),
            currency: "EUR".into(),
            period: Some(Period::Month),
        },
    )
}
struct Household {
    me: String,
    car: String,
    home: String,
    employer: String,
    insurer: String,
    policy: String,
}
/// Synthetic household described only with the shared vocabulary.
fn household(v: &mut Vault) -> Household {
    let me = v.profile_entity_id().unwrap();
    let car = v
        .create_entity(EntityKind::Vehicle, "Synthetic sedan")
        .unwrap();
    let home = v
        .create_entity(EntityKind::Address, "Synthetic street 1")
        .unwrap();
    let employer = v
        .create_entity(EntityKind::Organization, "Synthetic employer")
        .unwrap();
    let insurer = v
        .create_entity(EntityKind::Organization, "Synthetic insurer")
        .unwrap();
    let policy = v
        .create_entity(EntityKind::InsuranceContract, "Car policy")
        .unwrap();
    for draft in [
        link(&me, "person.residence", &home),
        link(&me, "person.employer", &employer),
        per_month(&me, "person.gross_income", "5000"),
        link(&me, "person.owns", &car),
        text(&home, "address.city", "Synthetic city"),
        text(&car, "vehicle.make", "Mercedes-Benz"),
        text(&car, "vehicle.model", "C-Class"),
        fact(
            &car,
            "vehicle.annual_mileage",
            FactValue::Quantity {
                amount: "15000".into(),
                unit: "km/year".into(),
            },
        ),
        link(&policy, "insurance.insured_object", &car),
        link(&policy, "contract.provider", &insurer),
        link(&policy, "contract.holder", &me),
        per_month(&policy, "contract.premium", "900"),
    ] {
        v.record_fact(&draft).unwrap();
    }
    Household {
        me,
        car,
        home,
        employer,
        insurer,
        policy,
    }
}
fn value<'a>(c: &'a EntityContext, subject: &str, property: &str) -> Option<&'a Value> {
    c.properties
        .iter()
        .find(|p| p.subject == subject && p.property == property)
        .map(|p| &p.resolution.facts[0].value)
}

#[test]
fn agent_context_reaches_everything_needed_to_compare_car_insurance() {
    let (_t, mut v) = setup();
    let h = household(&mut v);
    // From the person, the contract is only reachable through the incoming
    // link to the car; the insurer is one link further.
    let c = v.entity_context_at(&h.me, "2026-09-27", 3).unwrap();
    assert!(!c.truncated);
    let distance = |id: &str| c.entities.iter().find(|e| e.id == id).map(|e| e.distance);
    assert_eq!(distance(&h.me), Some(0));
    assert_eq!(distance(&h.car), Some(1));
    assert_eq!(distance(&h.home), Some(1));
    assert_eq!(distance(&h.employer), Some(1));
    assert_eq!(distance(&h.policy), Some(1)); // holder link
    assert_eq!(distance(&h.insurer), Some(2));
    assert_eq!(
        value(&c, &h.policy, "contract.premium"),
        Some(&json!({"amount":"900","currency":"EUR","period":"month"}))
    );
    assert_eq!(
        value(&c, &h.car, "vehicle.annual_mileage"),
        Some(&json!({"amount":"15000","unit":"km/year"}))
    );
    assert_eq!(value(&c, &h.car, "vehicle.model"), Some(&json!("C-Class")));
    assert_eq!(
        value(&c, &h.home, "address.city"),
        Some(&json!("Synthetic city"))
    );
    assert_eq!(
        value(&c, &h.me, "person.gross_income"),
        Some(&json!({"amount":"5000","currency":"EUR","period":"month"}))
    );
    assert_eq!(
        value(&c, &h.policy, "insurance.insured_object"),
        Some(&json!(h.car))
    );
    // Starting at the car still finds its contract through the incoming link.
    let from_car = v.entity_context_at(&h.car, "2026-09-27", 1).unwrap();
    assert!(from_car.entities.iter().any(|e| e.id == h.policy));
    assert!(
        from_car
            .entities
            .iter()
            .all(|e| e.distance <= 1 && e.id != h.insurer)
    );
    assert!(v.entity_context_at(&h.me, "2026-09-27", 9).is_err());
    let depth_zero = v.entity_context_at(&h.me, "2026-09-27", 0).unwrap();
    assert_eq!(depth_zero.entities.len(), 1);
    assert!(value(&depth_zero, &h.me, "person.owns").is_some());
}

#[test]
fn agent_context_follows_time_so_a_sold_car_and_old_premium_drop_out() {
    let (_t, mut v) = setup();
    let h = household(&mut v);
    let old_car = v.create_entity(EntityKind::Vehicle, "Sold car").unwrap();
    v.record_fact(&AssertionDraft {
        validity: Validity::Interval {
            from: "2020-01-01".into(),
            to: Some("2025-01-01".into()),
        },
        ..link(&h.me, "person.owns", &old_car)
    })
    .unwrap();
    let now = v.entity_context_at(&h.me, "2026-09-27", 2).unwrap();
    assert!(!now.entities.iter().any(|e| e.id == old_car));
    let then = v.entity_context_at(&h.me, "2022-06-01", 2).unwrap();
    assert!(then.entities.iter().any(|e| e.id == old_car));
    // Nothing about the policy was valid yet in 2022.
    assert!(!then.entities.iter().any(|e| e.id == h.policy));
}

#[test]
fn links_follow_merges_and_never_point_at_deleted_entities() {
    let (_t, mut v) = setup();
    let h = household(&mut v);
    let duplicate = v.create_entity(EntityKind::Vehicle, "My car").unwrap();
    v.merge_entities(&h.car, &duplicate).unwrap();
    let insured = v
        .resolve_fact_at(&h.policy, "insurance.insured_object", "2026-09-27")
        .unwrap();
    assert_eq!(insured.status, ResolutionStatus::Resolved);
    assert_eq!(insured.facts[0].value, json!(duplicate));
    let c = v.entity_context_at(&h.me, "2026-09-27", 3).unwrap();
    assert!(c.entities.iter().any(|e| e.id == duplicate));
    assert!(!c.entities.iter().any(|e| e.id == h.car));

    v.delete_entity(&h.insurer).unwrap();
    let provider = v
        .resolve_fact_at(&h.policy, "contract.provider", "2026-09-27")
        .unwrap();
    assert_eq!(provider.status, ResolutionStatus::Missing);
    let c = v.entity_context_at(&h.me, "2026-09-27", 3).unwrap();
    assert!(!c.entities.iter().any(|e| e.id == h.insurer));
    assert!(
        v.resolve_fact_at(&h.policy, "contract.undefined", "2026-09-27")
            .is_err()
    );
}

#[test]
fn vocabulary_rejects_links_between_the_wrong_kinds_of_entities() {
    let (_t, mut v) = setup();
    let h = household(&mut v);
    let before = last(&v);
    // A vehicle has no employer and a person is not an insured contract.
    assert!(
        v.record_fact(&link(&h.car, "person.employer", &h.employer))
            .is_err()
    );
    assert!(
        v.record_fact(&link(&h.me, "person.residence", &h.car))
            .is_err()
    );
    assert!(
        v.record_fact(&per_month(&h.me, "contract.premium", "1"))
            .is_err()
    );
    assert_eq!(last(&v), before);
    // Redefining a term with different kinds changes its meaning.
    let mut residence = vocabulary().find(|p| p.key == "person.residence").unwrap();
    assert!(v.define_property(&residence).is_ok());
    residence.object_kinds = vec![EntityKind::Organization];
    assert!(v.define_property(&residence).is_err());
    assert!(
        v.define_property(&PropertyDefinition {
            key: "custom.note".into(),
            label: "Note".into(),
            value_type: "text".into(),
            many: false,
            subject_kinds: Vec::new(),
            object_kinds: vec![EntityKind::Vehicle],
        })
        .is_err()
    );
}

#[test]
fn vocabulary_is_installed_once_and_preserves_existing_definitions() {
    let (t, v) = setup();
    let count = |v: &Vault| -> i64 {
        v.db.query_row(
            "SELECT count(*) FROM domain_revision WHERE record_kind='property_definition'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    let installed = count(&v);
    assert!(installed >= PERSONAL_VOCABULARY.len() as i64);
    v.db.execute(
        "UPDATE property_definition SET label='Mine',rules_json='{}' WHERE key='vehicle.model'",
        [],
    )
    .unwrap();
    v.db.execute(
        "DELETE FROM property_definition WHERE key='vehicle.make'",
        [],
    )
    .unwrap();
    let edits = count(&v);
    drop(v);
    let v = Vault::unlock(&t.path().join("vault"), PASSWORD).unwrap();
    let label: String =
        v.db.query_row(
            "SELECT label FROM property_definition WHERE key='vehicle.model'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(label, "Mine");
    // Only the missing term is restored; reopening is otherwise silent.
    assert_eq!(count(&v), edits + 1);
    drop(v);
    let v = Vault::unlock(&t.path().join("vault"), PASSWORD).unwrap();
    assert_eq!(count(&v), edits + 1);
}

#[test]
fn recurring_money_keeps_its_period_and_legacy_amounts_are_unchanged() {
    let (_t, mut v) = setup();
    let h = household(&mut v);
    let premium = v
        .resolve_fact_at(&h.policy, "contract.premium", "2026-09-27")
        .unwrap();
    assert_eq!(premium.facts[0].value["period"], json!("month"));
    // The same amount per year is a different fact, not a duplicate.
    let yearly = AssertionDraft {
        value: FactValue::Money {
            amount: "900".into(),
            currency: "EUR".into(),
            period: Some(Period::Year),
        },
        ..per_month(&h.policy, "contract.premium", "900")
    };
    assert_ne!(v.record_fact(&yearly).unwrap(), premium.facts[0].id);
    let stored: FactValue =
        serde_json::from_value(json!({"type":"money","value":{"amount":"1","currency":"EUR"}}))
            .unwrap();
    assert_eq!(
        stored,
        FactValue::Money {
            amount: "1".into(),
            currency: "EUR".into(),
            period: None
        }
    );
    assert!(
        v.collection("", false)
            .unwrap()
            .items
            .iter()
            .any(|i| matches!(&i.content,Content::Note(s) if s=="900 EUR per month"))
    );
}
