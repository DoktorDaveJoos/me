//! Isolated synthetic data for native filter UI checks.
use me_core::Vault;
fn main() {
    let root = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("Pass an empty test directory"),
    );
    let mut vault = Vault::create(&root.join("vault"), "synthetic-ui-password").unwrap();
    vault.set_automatic_evaluation(false).unwrap();
    for (label, value) in [
        ("Steuer-ID", "01234567890"),
        ("Geburtsdatum", "01.02.1990"),
        ("Social insurance number", "12010290B123"),
        ("IBAN", "DE89 3704 0044 0532 0130 00"),
        ("Email address", "erika@example.test"),
        ("Phone number", "+49 30 5550 1234"),
    ] {
        vault.save_note(None, label, value).unwrap();
    }
    let ids = vault
        .data_facts()
        .unwrap()
        .iter()
        .filter(|f| ["Tax ID", "Date of birth", "IBAN"].contains(&f.label.as_str()))
        .map(|f| f.id.clone())
        .collect::<Vec<_>>();
    vault.remember_data(&ids).unwrap();
    println!("Synthetic filter fixture ready.");
}
