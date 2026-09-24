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
