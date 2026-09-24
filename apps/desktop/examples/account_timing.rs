//! Synthetic release-mode timings. Optional loopback API must use a disposable database.
#![allow(dead_code)]
#[path = "../src/account_client.rs"]
mod account_client;
use me_core::{Vault, account};
use std::time::Instant;
fn timed<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let result = f();
    println!("{label}: {:.0} ms", start.elapsed().as_secs_f64() * 1000.);
    result
}
fn main() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("vault");
    let email = format!("timing-{}@example.test", uuid::Uuid::new_v4());
    let password = "synthetic timing password";
    let code = account::generate_recovery_code().unwrap();
    let request = timed("Prepare recovery step", || {
        account::prepare_registration(&root, &email, password, &code).unwrap()
    });
    timed("Stage local vault", || {
        account::stage_registration(&root, password, &request).unwrap()
    });
    let response = if std::env::var_os("ME_ACCOUNT_API_URL").is_some() {
        let server = account_client::server().unwrap();
        assert!(
            server.starts_with("http://127.0.0.1:"),
            "Use a disposable loopback API"
        );
        let registered = timed("Register HTTP", || {
            account_client::register(&server, &request).unwrap()
        });
        let signed_in = timed("Sign in (client derivation + HTTP)", || {
            account_client::sign_in(&server, &email, password).unwrap()
        });
        assert_eq!(registered.account, signed_in.account);
        signed_in
    } else {
        me_protocol::AccountResponse {
            account: me_protocol::Account {
                id: uuid::Uuid::new_v4().to_string(),
                email,
                email_verified: false,
                revision: 1,
            },
            vault: request.vault.clone(),
        }
    };
    timed("Envelope validation", || {
        account::validate_password_envelope(password, &response).unwrap()
    });
    timed("Open account vault", || {
        drop(
            account::open_account(&root, password, "http://127.0.0.1:8787", &response, false)
                .unwrap(),
        )
    });
    timed("Offline unlock", || {
        drop(Vault::unlock(&root, password).unwrap())
    });
}
