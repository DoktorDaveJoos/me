//! Synthetic fixture for manual native UI checks. Never opens the user's vault.
use me_core::{DocumentClass, ExtractedFact, ExtractionOutput, RejectedFact, Vault};
fn main() {
    let root = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("Pass an empty test directory"),
    );
    let mut vault = Vault::create(&root.join("vault"), "synthetic-ui-password").unwrap();
    vault.set_automatic_evaluation(false).unwrap();
    for (name, text, property, value, uncertain) in [
        (
            "Tax certificate.txt",
            "Erika Beispiel\nSteuer-ID: 01234567890",
            "person.tax_id",
            "01234567890",
            false,
        ),
        (
            "Insurance letter.txt",
            "Erika Beispiel\nSozialversicherungsnummer: 12010290B123",
            "person.social_insurance_number",
            "12010290B123",
            true,
        ),
        (
            "Birth certificate.txt",
            "Erika Beispiel\nGeburtsdatum: 01.02.1990",
            "person.birth_date",
            "01.02.1990",
            true,
        ),
    ] {
        let path = root.join(name);
        std::fs::write(&path, text).unwrap();
        let id = vault
            .import_document(&path, name, DocumentClass::Personal)
            .unwrap();
        vault.begin_evaluation(id, false).unwrap();
        let input = vault.prepare_extraction(id, "synthetic").unwrap();
        let fact = ExtractedFact {
            context_quote: String::new(),
            property: property.into(),
            value: value.into(),
            quote: value.into(),
            subject_quote: "Erika Beispiel".into(),
            segment_id: input.segments[0].segment_id.clone(),
        };
        vault
            .finish_extraction_with_questions(
                &input,
                ExtractionOutput {
                    facts: if uncertain {
                        vec![]
                    } else {
                        vec![fact.clone()]
                    },
                },
                if uncertain {
                    vec![RejectedFact {
                        fact,
                        code: "subject_not_in_source",
                    }]
                } else {
                    vec![]
                },
            )
            .unwrap();
        vault.finish_evaluation(id, None).unwrap();
    }
    println!("Created synthetic UI fixture.");
}
