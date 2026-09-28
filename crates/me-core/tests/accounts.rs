use me_core::{Vault, account::*};
use me_protocol::{Account, AccountResponse};
const PASSWORD: &str = "synthetic master password";
const NEW_PASSWORD: &str = "a different synthetic phrase";
fn response(request: &me_protocol::Registration) -> AccountResponse {
    AccountResponse {
        account: Account {
            id: uuid::Uuid::new_v4().to_string(),
            email: request.email.clone(),
            email_verified: false,
            revision: 1,
        },
        vault: request.vault.clone(),
    }
}
#[test]
fn registration_recovers_same_vault_and_keeps_data_and_backup() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vault");
    let mut vault = Vault::create(&root, PASSWORD).unwrap();
    vault
        .save_note(None, "Account migration marker", "private synthetic note")
        .unwrap();
    drop(vault);
    let code = generate_recovery_code().unwrap();
    let request = prepare_registration(&root, " Person@Example.com ", PASSWORD, &code).unwrap();
    let current = response(&request);
    let vault = open_account(
        &root,
        PASSWORD,
        "https://accounts.example.com",
        &current,
        false,
    )
    .unwrap();
    assert_eq!(vault.collection("", false).unwrap().items.len(), 1);
    assert_eq!(
        binding(&root).unwrap().unwrap().account.email,
        "person@example.com"
    );
    let backup = temp.path().join("saved.mebackup");
    vault.backup(&backup).unwrap();
    drop(vault);
    let next_code = generate_recovery_code().unwrap();
    assert!(
        prepare_recovery(
            &current,
            &generate_recovery_code().unwrap(),
            NEW_PASSWORD,
            &next_code
        )
        .is_err()
    );
    let recovered = prepare_recovery(&current, &code, NEW_PASSWORD, &next_code).unwrap();
    let next = AccountResponse {
        account: Account {
            revision: 2,
            ..current.account
        },
        vault: recovered.vault.clone(),
    };
    let vault = open_account(
        &root,
        NEW_PASSWORD,
        "https://accounts.example.com",
        &next,
        false,
    )
    .unwrap();
    assert_eq!(vault.collection("", false).unwrap().items.len(), 1);
    drop(vault);
    assert!(Vault::unlock(&root, PASSWORD).is_err());
    assert!(Vault::unlock(&root, NEW_PASSWORD).is_ok());
    assert!(
        prepare_recovery(
            &next,
            &code,
            NEW_PASSWORD,
            &generate_recovery_code().unwrap()
        )
        .is_err()
    );
    assert!(
        prepare_recovery(
            &next,
            &next_code,
            PASSWORD,
            &generate_recovery_code().unwrap()
        )
        .is_ok()
    );
    // The original encrypted backup remains recoverable with its original password.
    let restored = temp.path().join("restored");
    let vault = Vault::restore(&backup, &restored, PASSWORD).unwrap();
    assert_eq!(vault.collection("", false).unwrap().items.len(), 1);
    drop(vault);
    let vault = open_account(
        &restored,
        NEW_PASSWORD,
        "https://accounts.example.com",
        &next,
        false,
    )
    .unwrap();
    assert_eq!(vault.collection("", false).unwrap().items.len(), 1);
}
#[test]
fn registration_can_resume_after_network_commit_without_losing_keys() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vault");
    let code = generate_recovery_code().unwrap();
    let request = prepare_registration(&root, "person@example.com", PASSWORD, &code).unwrap();
    assert!(!root.exists());
    stage_registration(&root, PASSWORD, &request).unwrap();
    stage_registration(&root, PASSWORD, &request).unwrap();
    let response = response(&request);
    assert!(binding(&root).unwrap().is_none());
    let vault = open_account(
        &root,
        PASSWORD,
        "https://accounts.example.com",
        &response,
        false,
    )
    .unwrap();
    drop(vault);
    assert!(binding(&root).unwrap().is_some());
    assert!(prepare_registration(&root, "person@example.com", PASSWORD, &code).is_err());
}
#[test]
fn wrong_account_corruption_and_stale_response_never_overwrite_local_keys() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vault");
    let code = generate_recovery_code().unwrap();
    let request = prepare_registration(&root, "person@example.com", PASSWORD, &code).unwrap();
    let current = response(&request);
    drop(
        open_account(
            &root,
            PASSWORD,
            "https://accounts.example.com",
            &current,
            true,
        )
        .unwrap(),
    );
    let original = std::fs::read(root.join("header.json")).unwrap();
    assert!(
        open_account(
            &root,
            "wrong password",
            "https://accounts.example.com",
            &current,
            false
        )
        .is_err()
    );
    let mut bad = current.clone();
    bad.vault.wrapped_keys[60] ^= 1;
    assert!(open_account(&root, PASSWORD, "https://accounts.example.com", &bad, false).is_err());
    let mut bad = current.clone();
    bad.vault.vault_id = uuid::Uuid::new_v4().to_string();
    assert!(open_account(&root, PASSWORD, "https://accounts.example.com", &bad, false).is_err());
    let mut bad = current.clone();
    bad.account.id = uuid::Uuid::new_v4().to_string();
    assert!(open_account(&root, PASSWORD, "https://accounts.example.com", &bad, false).is_err());
    assert!(
        open_account(
            &root,
            PASSWORD,
            "https://another.example.com",
            &current,
            false
        )
        .is_err()
    );
    assert_eq!(original, std::fs::read(root.join("header.json")).unwrap());
    let mut stale = current.clone();
    stale.account.revision = 0;
    assert!(
        open_account(
            &root,
            PASSWORD,
            "https://accounts.example.com",
            &stale,
            false
        )
        .is_err()
    );
    let mut wrong_target = current.clone();
    wrong_target.vault.vault_id = uuid::Uuid::new_v4().to_string();
    assert!(check_recovery_target(&root, &wrong_target).is_err());
    assert!(check_recovery_target(&root, &current).is_ok());
    let absent = temp.path().join("other-device");
    assert!(
        open_account(
            &absent,
            PASSWORD,
            "https://accounts.example.com",
            &current,
            false
        )
        .is_err()
    );
    assert!(!absent.exists());
}
#[test]
fn authentication_and_recovery_proofs_are_separate_from_wrapping_keys() {
    let a = authentication_secret("Person@Example.com", PASSWORD).unwrap();
    assert_eq!(
        *a,
        *authentication_secret("person@example.com", PASSWORD).unwrap()
    );
    assert_ne!(
        *a,
        *authentication_secret("another@example.com", PASSWORD).unwrap()
    );
    let code = generate_recovery_code().unwrap();
    assert_eq!(code.len(), 71);
    assert_eq!(
        *recovery_secret(&code).unwrap(),
        *recovery_secret(&code.to_ascii_lowercase().replace('-', " ")).unwrap()
    );
    assert!(recovery_secret("wrong").is_err());
    let root = tempfile::tempdir().unwrap();
    let registration = prepare_registration(
        &root.path().join("vault"),
        "person@example.com",
        PASSWORD,
        &code,
    )
    .unwrap();
    let wire = serde_json::to_string(&registration).unwrap();
    assert!(!wire.contains(PASSWORD));
    assert!(!wire.contains(code.as_str()));
}
const STRONG_PASSWORD: &str = "violet harbor quantum dusk 47";
#[test]
fn new_master_passwords_must_resist_offline_guessing() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vault");
    let code = generate_recovery_code().unwrap();
    let email = "kvothe.arliden@example.com";
    for weak in [
        "password1234",
        "Passwort1234",
        "Sommer2024!!",
        "qwertzuiop12",
        "kvothearliden1",
        "aaaaaaaaaaaaaaaa",
    ] {
        assert!(
            matches!(
                check_new_master_password(weak, Some(email)),
                Err(me_core::Error::Validation(_))
            ),
            "{weak}"
        );
        assert!(
            prepare_registration(&root, email, weak, &code).is_err(),
            "{weak}"
        );
    }
    check_new_master_password(STRONG_PASSWORD, Some(email)).unwrap();
    prepare_registration(&root, email, STRONG_PASSWORD, &code).unwrap();
}
#[test]
fn recovery_requires_strong_password_while_existing_vaults_still_attach() {
    // Attaching keeps the current vault password so existing data is never stranded.
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vault");
    drop(Vault::create(&root, "abcdefghij").unwrap());
    let code = generate_recovery_code().unwrap();
    let request = prepare_registration(&root, "person@example.com", "abcdefghij", &code).unwrap();
    let current = response(&request);
    let next = generate_recovery_code().unwrap();
    assert!(matches!(
        prepare_recovery(&current, &code, "abcdefghijk", &next),
        Err(me_core::Error::Validation(_))
    ));
    prepare_recovery(&current, &code, STRONG_PASSWORD, &next).unwrap();
}
const COMPOSED: &str = "Grüße aus Köln am Rhein";
fn decomposed() -> String {
    COMPOSED.replace('ü', "u\u{308}").replace('ö', "o\u{308}")
}
#[test]
fn composed_and_decomposed_password_input_derive_the_same_keys() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vault");
    let nfd = decomposed();
    assert_ne!(nfd, COMPOSED);
    drop(Vault::create(&root, &nfd).unwrap());
    drop(Vault::unlock(&root, COMPOSED).unwrap());
    drop(Vault::unlock(&root, &nfd).unwrap());
    let email = "person@example.com";
    assert_eq!(
        *authentication_secret(email, COMPOSED).unwrap(),
        *authentication_secret(email, &nfd).unwrap()
    );
    assert!(
        legacy_authentication_secret(email, COMPOSED)
            .unwrap()
            .is_none()
    );
    let legacy = legacy_authentication_secret(email, &nfd).unwrap().unwrap();
    assert_ne!(*legacy, *authentication_secret(email, &nfd).unwrap());
}
#[test]
fn authentication_secret_matches_pre_normalization_vectors() {
    // Computed by the release before normalization. Shared with future clients.
    let email = "vector@example.test";
    assert_eq!(
        authentication_secret(email, "correct horse battery staple")
            .unwrap()
            .as_str(),
        "956e17ca15deb152712d857378a1b3e61853c55daa0218844281aca9e14fd399"
    );
    let decomposed = "Gru\u{308}ße aus Ko\u{308}ln";
    assert_eq!(
        legacy_authentication_secret(email, decomposed)
            .unwrap()
            .unwrap()
            .as_str(),
        "265fe6b629c4728f9ebb75b3620b0cf7be17129e8e0e8bceddef42ffaf1df7dd"
    );
    assert_eq!(
        *authentication_secret(email, decomposed).unwrap(),
        *authentication_secret(email, "Grüße aus Köln").unwrap()
    );
}
