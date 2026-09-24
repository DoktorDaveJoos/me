//! Real HTTP + PostgreSQL smoke test with synthetic credentials and a temp vault.
//! Requires a disposable running ME account API via ME_ACCOUNT_API_URL.
#![allow(dead_code)]
#[path = "../src/account_client.rs"]
mod account_client;
use me_core::{Vault, account};
fn main() {
    let server = account_client::server().unwrap();
    assert!(
        server.starts_with("http://127.0.0.1:"),
        "Use only a disposable local service"
    );
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("vault");
    let email = format!("smoke-{}@example.test", uuid::Uuid::new_v4());
    let password = "synthetic account smoke phrase";
    let code = account::generate_recovery_code().unwrap();
    let request = account::prepare_registration(&root, &email, password, &code).unwrap();
    account::stage_registration(&root, password, &request).unwrap();
    let registered = account_client::register(&server, &request).unwrap();
    let retried = account_client::register(&server, &request).unwrap();
    assert_eq!(registered.account, retried.account);
    assert!(account_client::sign_in(&server, &email, "wrong password").is_err());
    let signed_in = account_client::sign_in(&server, &email, password).unwrap();
    let mut vault = account::open_account(&root, password, &server, &signed_in, false).unwrap();
    vault
        .save_note(
            None,
            "Synthetic roundtrip",
            "This data never goes to the account API.",
        )
        .unwrap();
    drop(vault);
    let recovery = account_client::recovery_account(&server, &email, &code).unwrap();
    let new_code = account::generate_recovery_code().unwrap();
    let new_password = "new synthetic account smoke phrase";
    let change = account::prepare_recovery(&recovery, &code, new_password, &new_code).unwrap();
    let recovered = account_client::recover(&server, &change).unwrap();
    // Same request after a lost response must reconcile with the new credential.
    let repeated = account_client::recover(&server, &change).unwrap();
    assert_eq!(recovered.account, repeated.account);
    let vault = account::open_account(&root, new_password, &server, &recovered, false).unwrap();
    assert_eq!(vault.collection("", false).unwrap().items.len(), 1);
    drop(vault);
    assert!(Vault::unlock(&root, password).is_err());
    assert!(Vault::unlock(&root, new_password).is_ok());
    assert!(account_client::recovery_account(&server, &email, &code).is_err());
    assert!(account_client::recovery_account(&server, &email, &new_code).is_ok());
    assert!(account_client::sign_in(&server, &email, password).is_err());
    assert!(account_client::sign_in(&server, &email, new_password).is_ok());
    println!(
        "PASS: real HTTP registration/retry, sign-in, recovery/retry, code rotation, local data preservation and offline unlock."
    );
}
