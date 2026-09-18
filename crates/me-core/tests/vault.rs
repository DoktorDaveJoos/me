use me_core::{Content, DocumentClass, Vault};
use std::{fs, path::Path};
const PASSWORD: &str = "synthetic-passphrase-2026";
fn create(temp: &tempfile::TempDir) -> Vault {
    Vault::create(&temp.path().join("vault"), PASSWORD).unwrap()
}
fn text(temp: &tempfile::TempDir) -> std::path::PathBuf {
    let path = temp.path().join("Bescheinigung.txt");
    fs::write(
        &path,
        "SYNTHETIC Erste Hilfe München; Kennnummer 001234-TEST",
    )
    .unwrap();
    path
}
fn no_plaintext(dir: &Path, secret: &[u8]) {
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            no_plaintext(&p, secret)
        } else {
            assert!(
                !fs::read(p)
                    .unwrap()
                    .windows(secret.len())
                    .any(|w| w == secret)
            );
        }
    }
}
#[test]
fn persistent_notes_preserve_identity_leading_zeroes_pins_and_revisions() {
    let t = tempfile::tempdir().unwrap();
    let mut v = create(&t);
    let note = v
        .save_note(None, "Versicherungsnummer", "001234-TEST")
        .unwrap();
    let stable = v
        .collection("", false)
        .unwrap()
        .get(note)
        .unwrap()
        .stable_id
        .clone();
    v.toggle_pin(note).unwrap();
    assert!(v.save_note(Some(note), " ", "broken").is_err());
    v.save_note(Some(note), "Versicherungsnummer", "009999-TEST")
        .unwrap();
    let history = v.history(note).unwrap();
    assert!(
        history
            .iter()
            .any(|r| r.value == "001234-TEST" && r.state == "retract")
    );
    drop(v);
    let v = Vault::unlock(&t.path().join("vault"), PASSWORD).unwrap();
    let c = v.collection("009999", true).unwrap();
    let saved = c.get(note).unwrap();
    assert_eq!(saved.stable_id, stable);
    assert!(saved.pinned);
    assert_eq!(saved.content, Content::Note("009999-TEST".into()));
    assert!(v.collection("001234", false).unwrap().items.is_empty());
    no_plaintext(&t.path().join("vault"), b"009999-TEST");
}
#[test]
fn wrong_password_and_plain_sqlite_cannot_read_the_database() {
    let t = tempfile::tempdir().unwrap();
    let mut v = create(&t);
    v.save_note(None, "Privat", "SYNTHETIC-SECRET-ABC").unwrap();
    drop(v);
    assert!(Vault::unlock(&t.path().join("vault"), "wrong password").is_err());
    let db = rusqlite::Connection::open(t.path().join("vault/vault.db")).unwrap();
    assert!(
        db.query_row("SELECT count(*) FROM sqlite_master", [], |r| r
            .get::<_, i64>(0))
            .is_err()
    );
    no_plaintext(&t.path().join("vault"), b"SYNTHETIC-SECRET-ABC");
}
#[test]
fn encrypted_import_is_durable_and_delivery_context_is_not_content_equality() {
    let t = tempfile::tempdir().unwrap();
    let mut v = create(&t);
    let path = text(&t);
    let first = v
        .import_document(&path, "delivery-1", DocumentClass::Unclassified)
        .unwrap();
    assert_eq!(
        first,
        v.import_document(&path, "delivery-1", DocumentClass::Unclassified)
            .unwrap()
    );
    assert_ne!(
        first,
        v.import_document(&path, "delivery-2", DocumentClass::Unclassified)
            .unwrap()
    );
    assert_eq!(v.collection("", false).unwrap().len(), 2);
    assert!(v.collection("Hilfe", false).unwrap().items.is_empty());
    assert_eq!(v.collection("Bescheinigung", false).unwrap().items.len(), 2);
    fs::remove_file(&path).unwrap();
    drop(v);
    let v = Vault::unlock(&t.path().join("vault"), PASSWORD).unwrap();
    let exported = t.path().join("export.txt");
    v.export_document(first, &exported).unwrap();
    assert!(
        fs::read_to_string(exported)
            .unwrap()
            .contains("001234-TEST")
    );
    no_plaintext(&t.path().join("vault"), b"Erste Hilfe");
}
#[test]
fn explicit_classification_enables_unicode_fulltext_and_credential_is_excluded() {
    let t = tempfile::tempdir().unwrap();
    let mut v = create(&t);
    let path = text(&t);
    let doc = v
        .import_document(&path, "normal", DocumentClass::Unclassified)
        .unwrap();
    let secret = v
        .import_document(&path, "secret", DocumentClass::Credential)
        .unwrap();
    assert!(v.enable_text_search(secret).is_err());
    v.enable_text_search(doc).unwrap();
    let found = v.collection("ERSTE münchen", false).unwrap();
    assert_eq!(found.items.len(), 1);
    assert_eq!(found.items[0].id, doc);
    for query in [
        "\" OR * NOT",
        "' ; DROP TABLE source; --",
        "\"",
        "NEAR(Hilfe)",
    ] {
        assert!(v.collection(query, false).is_ok());
    }
    assert_eq!(v.collection("", false).unwrap().len(), 2);
}
#[test]
fn encrypted_backup_restores_originals_notes_and_search_without_the_original_vault() {
    let t = tempfile::tempdir().unwrap();
    let mut v = create(&t);
    let path = text(&t);
    let doc = v
        .import_document(&path, "d", DocumentClass::Personal)
        .unwrap();
    v.save_note(None, "Schuhgröße", "42").unwrap();
    let backup = t.path().join("snapshot.mebackup");
    v.backup(&backup).unwrap();
    drop(v);
    fs::remove_dir_all(t.path().join("vault")).unwrap();
    fs::remove_file(path).unwrap();
    no_plaintext(&backup, b"Erste Hilfe");
    let restore = t.path().join("restored");
    let v = Vault::restore(&backup, &restore, PASSWORD).unwrap();
    assert_eq!(v.collection("Hilfe", false).unwrap().items.len(), 1);
    assert_eq!(v.collection("42", false).unwrap().items.len(), 1);
    v.export_document(doc, &t.path().join("restored.txt"))
        .unwrap();
    assert!(
        fs::read_to_string(t.path().join("restored.txt"))
            .unwrap()
            .contains("SYNTHETIC")
    );
}
#[test]
fn tampered_object_is_never_exported_or_acknowledged_as_a_backup() {
    let t = tempfile::tempdir().unwrap();
    let mut v = create(&t);
    let doc = v
        .import_document(&text(&t), "d", DocumentClass::Unclassified)
        .unwrap();
    let object = fs::read_dir(t.path().join("vault/objects"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut bytes = fs::read(&object).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    fs::write(object, bytes).unwrap();
    let out = t.path().join("export.txt");
    assert!(v.export_document(doc, &out).is_err());
    assert!(!out.exists());
    let backup = t.path().join("backup");
    assert!(v.backup(&backup).is_err());
    assert!(!backup.exists());
}
#[test]
fn operations_never_clobber_an_existing_vault_export_or_backup() {
    let t = tempfile::tempdir().unwrap();
    let mut v = create(&t);
    let doc = v
        .import_document(&text(&t), "d", DocumentClass::Unclassified)
        .unwrap();
    assert!(Vault::create(&t.path().join("vault"), PASSWORD).is_err());
    let existing = t.path().join("existing.txt");
    fs::write(&existing, "retain").unwrap();
    assert!(v.export_document(doc, &existing).is_err());
    assert_eq!(fs::read_to_string(&existing).unwrap(), "retain");
    let backup = t.path().join("backup");
    v.backup(&backup).unwrap();
    assert!(v.backup(&backup).is_err());
    assert!(backup.join("header.json").exists());
    assert!(Vault::restore(&backup, &t.path().join("vault"), PASSWORD).is_err());
}
#[test]
fn second_process_cannot_open_a_live_vault_and_lock_is_released_on_drop() {
    let t = tempfile::tempdir().unwrap();
    let v = create(&t);
    assert!(matches!(
        Vault::unlock(&t.path().join("vault"), PASSWORD),
        Err(me_core::Error::InUse)
    ));
    drop(v);
    assert!(Vault::unlock(&t.path().join("vault"), PASSWORD).is_ok());
}
#[test]
fn corrupt_or_partial_vault_is_not_treated_as_new() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("broken");
    fs::create_dir(&root).unwrap();
    assert!(Vault::exists(&root).is_err());
    assert!(!Vault::exists(&t.path().join("missing")).unwrap());
    assert!(Vault::create(&root, PASSWORD).is_err());
    assert!(root.exists());
}
#[test]
fn restore_with_wrong_password_leaves_destination_absent_and_backup_intact() {
    let t = tempfile::tempdir().unwrap();
    let v = create(&t);
    let backup = t.path().join("backup");
    v.backup(&backup).unwrap();
    let restore = t.path().join("restore");
    assert!(Vault::restore(&backup, &restore, "incorrect").is_err());
    assert!(!restore.exists());
    assert!(backup.join("header.json").exists());
}

#[test]
fn ten_lowercase_characters_can_create_and_unlock_a_vault() {
    let t = tempfile::tempdir().unwrap();
    let too_short = t.path().join("too-short");
    assert!(matches!(
        Vault::create(&too_short, "abcdefghi"),
        Err(me_core::Error::Validation(_))
    ));
    assert!(!too_short.exists());

    let root = t.path().join("vault");
    let mut vault = Vault::create(&root, "abcdefghij").unwrap();
    vault
        .save_note(None, "Synthetic", "password policy regression")
        .unwrap();
    drop(vault);

    let vault = Vault::unlock(&root, "abcdefghij").unwrap();
    assert_eq!(vault.collection("regression", false).unwrap().total, 1);
}
