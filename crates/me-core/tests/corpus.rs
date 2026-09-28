//! The synthetic document corpus (`fixtures/documents`, invented values only).
//! `expectations.json` lists, per fixture, every value the local scanners count
//! as worth reading (checksum identifiers, labeled amounts and labeled dates) and
//! every profile value, with its label, a verbatim quote, the expected slot,
//! period and owner. The opt-in live evaluation in `me-agent` measures the real
//! reader and TypeSafe against the same expectations.
use me_core::{
    SourceSegment, TypingContext, ValueKind, doc_type, find_candidates, locate, type_value,
    uncovered,
};
use serde_json::Value;

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/documents");
/// Owners as stored on document facts; `null` in a fixture means "not judged".
const OWNERS: &[&str] = &["self", "household", "party", "organization"];
/// Periods as stored on document facts (`none` = not periodic).
const PERIODS: &[&str] = &["document", "cumulative", "other", "none"];

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{DIR}/{name}")).unwrap()
}

#[test]
fn every_fixture_has_expectations() {
    let expectations: Value = serde_json::from_str(&fixture("expectations.json")).unwrap();
    let mut fixtures = 0;
    for entry in std::fs::read_dir(DIR).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        if name.ends_with(".txt") {
            fixtures += 1;
            assert!(expectations.get(&name).is_some(), "{name}: no expectations");
        }
    }
    assert_eq!(fixtures, expectations.as_object().unwrap().len());
}

#[test]
fn every_fixture_value_worth_reading_is_expected_and_every_slot_value_types() {
    let expectations: Value = serde_json::from_str(&fixture("expectations.json")).unwrap();
    for (name, spec) in expectations.as_object().unwrap() {
        let text = fixture(name);
        let segments = [SourceSegment {
            id: "s0",
            text: &text,
        }];
        let key = spec["doc_type"].as_str().unwrap();
        let kind = doc_type(key).unwrap_or_else(|| panic!("{name}: unknown document type {key}"));
        let ctx = TypingContext::new(&segments, Some(key), 2026);
        let mut covered = Vec::new();
        for f in spec["facts"].as_array().unwrap() {
            let (label, value, quote) = (
                f["label"].as_str().unwrap(),
                f["value"].as_str().unwrap(),
                f["quote"].as_str().unwrap(),
            );
            let at = locate(&segments, "s0", quote, value)
                .unwrap_or_else(|| panic!("{name}: {quote:?} does not hold {value:?} verbatim"));
            covered.push(("s0".to_owned(), at.quote_start, at.quote_end));
            for (field, allowed) in [("owner", OWNERS), ("period", PERIODS)] {
                if let Some(v) = f[field].as_str() {
                    assert!(allowed.contains(&v), "{name}: {label}: {field} {v}");
                }
            }
            let Some(slot) = f["slot"].as_str() else {
                continue;
            };
            let slot = kind
                .slot(slot)
                .unwrap_or_else(|| panic!("{name}: {key} has no slot {slot}"));
            let typed = type_value(&ctx, &at, label, value, slot.value).is_some();
            // A printed category that is not a code (a tariff name) is judged by
            // TypeSafe; its expected key is recorded instead.
            let judged = matches!(slot.value, ValueKind::Category(_)) && f["category"].is_string();
            assert!(
                typed || judged,
                "{name}: {value:?} does not type as {}",
                slot.key
            );
        }
        let open = uncovered(
            &find_candidates(&segments, &["Max Beispiel"], 2026),
            &covered,
        );
        assert!(
            open.is_empty(),
            "{name}: expectations miss {:?}",
            open.iter().map(|u| (&u.label, &u.text)).collect::<Vec<_>>()
        );
    }
}
