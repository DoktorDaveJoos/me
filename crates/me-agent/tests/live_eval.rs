//! Opt-in live evaluation of deep-first reading on the synthetic corpus in
//! `crates/me-core/fixtures/documents` (invented values only; never a vault or
//! personal data). Every fixture runs through the steps the desktop runs in
//! `apps/desktop/src/read_import.rs`, without a vault: Classify (TypeSafe), the
//! guided reader and at most one omission audit (OpenAI through the signed-in ME.
//! Codex home), local typing, TypeSafe verification and profile values. Results
//! are compared with `expectations.json` and printed per family. The numbers are
//! a baseline, not a gate: the test asserts only that every document completed.
//!
//! It lives in `tests/` on purpose: unit-test builds of `me-agent` replace the
//! reader's own TypeSafe section checks with a fake and would not measure the
//! production reader.
//!
//! Run manually; it spends ChatGPT usage and TypeSafe requests:
//!
//! ```sh
//! ME_CODEX_TEST_HOME=<signed-in ME. Codex home> \
//!   ./scripts/cargo test -p me-agent --test live_eval -- --ignored --nocapture
//! ```
//!
//! TypeSafe is configured as for the app: `TYPESAFE_API_KEY`,
//! `ME_TYPESAFE_ENV_FILE` or the repository's private `.env`.
use me_agent::codex::{self, NoCheckpoints, Progress};
use me_agent::fact_verification::verify_facts;
use me_agent::graph_pipeline::{Classification, Document, classify};
use me_agent::guides::reading_guide;
use me_agent::read_assembly::{
    apply_direction, covered_spans, keep_grounded, named_owners, profile_values, read_fact,
    time_year, type_facts,
};
use me_agent::typesafe::{DecisionResponse, Decisions, TypeSafe};
use me_core::{
    CandidateValue, DEFAULT_CHECK_THRESHOLD, DocType, ExtractionInput, ExtractionOutput,
    HouseholdMember, IdentityAnchors, ImportProvider, Period, ReadFact, SlotContent, SlotValue,
    SourceSegment, SourceText, Tier, TypingContext, ValueKind, category_key, doc_type,
    find_candidates, locate, type_value, uncovered,
};
use serde_json::Value;
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::AtomicBool};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../me-core/fixtures/documents");

/// The synthetic identity: Max Beispiel and his partner Erika Beispiel.
fn anchors() -> IdentityAnchors {
    IdentityAnchors {
        self_entity: "00000000-0000-4000-8000-000000000001".into(),
        self_name: "Max Beispiel".into(),
        aliases: vec![],
        birth_date: None,
        household: vec![HouseholdMember {
            entity: "00000000-0000-4000-8000-000000000002".into(),
            name: "Erika Beispiel".into(),
            role: "partner".into(),
        }],
        setup_complete: true,
    }
}

/// The vault's segmentation (`me-core` `documents.rs`): at most 1,600 bytes,
/// ending after the last line break past 800 bytes.
fn segments(text: &str) -> Vec<SourceText> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + 1600).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        if end < text.len()
            && let Some(newline) = text[start..end].rfind('\n')
            && newline > 800
        {
            end = start + newline + 1;
        }
        out.push(SourceText {
            segment_id: format!("s{}", out.len() + 1),
            ordinal: out.len() as i64 + 1,
            text: text[start..end].to_owned(),
        });
        start = end;
    }
    out
}

/// TypeSafe with a request count.
struct Counted {
    inner: TypeSafe,
    requests: u32,
}
impl Decisions for Counted {
    fn evaluate(
        &mut self,
        state: Value,
        questions: Value,
        cancel: &AtomicBool,
    ) -> me_agent::typesafe::Result<DecisionResponse> {
        self.requests += 1;
        self.inner.evaluate(state, questions, cancel)
    }
}

/// What one read produced.
struct Outcome {
    classification: Classification,
    values: Vec<SlotValue>,
    facts: Vec<ReadFact>,
    uninterpreted: usize,
    reader_requests: u32,
    typesafe_requests: u32,
}

/// One document end to end, as `read_import::read` does it.
fn read(
    home: &Path,
    title: &str,
    text: &str,
    anchors: &IdentityAnchors,
    typesafe: &mut Counted,
) -> Result<Outcome, String> {
    let year = time_year();
    let input = ExtractionInput {
        run_id: format!("live-eval-{title}"),
        source_id: format!("synthetic-{title}"),
        title: title.to_owned(),
        segments: segments(text),
    };
    let segments: Vec<SourceSegment<'_>> = input
        .segments
        .iter()
        .map(|s| SourceSegment {
            id: &s.segment_id,
            text: &s.text,
        })
        .collect();
    let names = anchors.names();
    let name_refs: Vec<&str> = names.iter().map(|(_, n)| n.as_str()).collect();
    let candidates = find_candidates(&segments, &name_refs, year);
    let cancel = Arc::new(AtomicBool::new(false));
    let before = typesafe.requests;
    let (reader, internal) = (Cell::new(0u32), Cell::new(0u32));
    let count = |p: Progress| {
        if let Progress::Request { provider } = p {
            let n = match provider {
                ImportProvider::OpenAi => &reader,
                ImportProvider::TypeSafe => &internal,
                ImportProvider::Local => return,
            };
            n.set(n.get() + 1);
        }
    };

    // Classify.
    let document = Document {
        title,
        segments: &segments,
    };
    let classification =
        classify(document, anchors, &candidates, typesafe, &cancel).map_err(|f| f.message)?;
    let kind = classification.eager_type();
    let doc_label = classification
        .doc_type
        .as_deref()
        .and_then(doc_type)
        .map_or("Document", |t| t.label);
    let guide = reading_guide(&classification.family, classification.doc_type.as_deref());

    // Guided reading, then the omission sweep with at most one focused audit.
    let report = codex::extract_document(
        home,
        &input,
        &guide,
        cancel.clone(),
        count,
        &mut NoCheckpoints,
    )?;
    let mut facts = report.output.facts;
    let mut rejected = BTreeMap::new();
    keep_grounded(&mut facts, &mut rejected, report.rejected);
    let mut open = uncovered(&candidates, &covered_spans(&segments, &facts));
    if !open.is_empty() {
        let previous = ExtractionOutput {
            facts: facts.clone(),
        };
        let audit = codex::audit_document(
            home,
            &input,
            &guide,
            &previous,
            &open,
            cancel.clone(),
            count,
            &mut NoCheckpoints,
        )?;
        for f in audit.output.facts {
            me_core::merge_extracted_fact(&mut facts, f);
        }
        keep_grounded(&mut facts, &mut rejected, audit.rejected);
        open = uncovered(&candidates, &covered_spans(&segments, &facts));
    }

    // Local typing and TypeSafe verification.
    let typing = TypingContext::new(&segments, classification.doc_type.as_deref(), year);
    let (typed, inputs) = type_facts(&facts, &segments, &typing, kind);
    let subject = classification
        .subject
        .clone()
        .or_else(|| Some(anchors.self_entity.clone()));
    let excerpt = segments.first().map_or("", |s| s.text);
    let verification = verify_facts(
        title,
        doc_label,
        excerpt,
        &inputs,
        anchors,
        &named_owners(&candidates),
        subject.as_deref(),
        DEFAULT_CHECK_THRESHOLD,
        typesafe,
        &cancel,
    )
    .map_err(|f| f.message)?;
    let values = kind
        .map(|k| {
            profile_values(
                k,
                &candidates,
                &segments,
                &typed,
                &inputs,
                &verification.verdicts,
            )
        })
        .unwrap_or_default();
    let facts = typed
        .iter()
        .zip(&inputs)
        .zip(&verification.verdicts)
        .map(|((t, i), v)| read_fact(t, i, v, &anchors.self_entity))
        .collect();
    Ok(Outcome {
        classification,
        values,
        facts,
        uninterpreted: open.len(),
        reader_requests: reader.get(),
        typesafe_requests: internal.get() + typesafe.requests - before,
    })
}

/// The profile value an expected slot fact should produce.
enum Want {
    Value(Box<CandidateValue>),
    Key(String),
    Text(String),
}

struct Expected {
    label: String,
    value: String,
    slot: Option<String>,
    period: Option<String>,
    owner: Option<String>,
    /// Segment and byte span of the printed value.
    at: (String, usize, usize),
    want: Option<Want>,
    payment_period: Option<Period>,
}

fn norm(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn expectations(spec: &Value, segments: &[SourceSegment<'_>], kind: &DocType) -> Vec<Expected> {
    let ctx = TypingContext::new(segments, Some(kind.key), time_year());
    let text = |f: &Value, key: &str| f[key].as_str().map(str::to_owned);
    spec["facts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            let (label, value, quote) = (
                text(f, "label").unwrap(),
                text(f, "value").unwrap(),
                text(f, "quote").unwrap(),
            );
            let located = segments
                .iter()
                .find_map(|s| locate(segments, s.id, &quote, &value))
                .unwrap_or_else(|| panic!("{quote:?} is not inside one segment"));
            let slot = text(f, "slot")
                .and_then(|key| kind.slot(&key))
                .filter(|_| kind.tier == Tier::Eager);
            let want = slot.map(|slot| match slot.value {
                ValueKind::Name | ValueKind::Text => Want::Text(norm(&value)),
                ValueKind::Category(options) => Want::Key(
                    text(f, "category")
                        .or_else(|| category_key(options, &value).map(str::to_owned))
                        .expect("category"),
                ),
                kind => {
                    let mut c = type_value(&ctx, &located, &label, &value, kind).expect("typed");
                    if kind == ValueKind::Balance {
                        apply_direction(&mut c, Some(f["direction"] == "refund"));
                    }
                    Want::Value(Box::new(c.value))
                }
            });
            Expected {
                label,
                value,
                slot: slot.map(|s| s.key.to_owned()),
                period: text(f, "period"),
                owner: text(f, "owner"),
                at: (located.segment_id, located.start, located.end),
                want,
                payment_period: f
                    .get("payment_period")
                    .map(|p| serde_json::from_value(p.clone()).expect("period")),
            }
        })
        .collect()
}

fn holds(e: &Expected, v: &SlotValue) -> bool {
    let value = match (&e.want, &v.content) {
        (Some(Want::Value(want)), SlotContent::Candidate(c)) => c.value == **want,
        (Some(Want::Key(want)), SlotContent::Category { key }) => key == want,
        (Some(Want::Key(want)), SlotContent::Candidate(c)) => {
            matches!(&c.value, CandidateValue::Text(t) if t == want)
        }
        (Some(Want::Text(want)), SlotContent::Candidate(c)) => matches!(
            &c.value,
            CandidateValue::Text(t) | CandidateValue::Identifier(t) if norm(t) == *want
        ),
        _ => false,
    };
    value
        && e.slot.as_deref() == Some(v.slot.as_str())
        && e.payment_period.is_none_or(|p| v.period == Some(p))
}

fn reads(e: &Expected, f: &ReadFact) -> bool {
    let (segment, start, end) = &e.at;
    f.segment_id == *segment && f.start < *end && *start < f.end
}

#[derive(Default)]
struct Tally {
    documents: u32,
    type_right: u32,
    slots_expected: u32,
    slots_found: u32,
    slots_made: u32,
    slots_right: u32,
    facts_expected: u32,
    facts_found: u32,
    owners_judged: u32,
    owners_right: u32,
    periods_judged: u32,
    periods_right: u32,
    false_completions: u32,
    checks: u32,
    uninterpreted: u32,
    reader_requests: u32,
    typesafe_requests: u32,
}

fn ratio(n: u32, d: u32) -> String {
    if d == 0 {
        "n/a".into()
    } else {
        format!("{:.2} ({n}/{d})", f64::from(n) / f64::from(d))
    }
}
fn per_document(n: u32, d: u32) -> String {
    format!("{:.2}", f64::from(n) / f64::from(d.max(1)))
}

impl Tally {
    /// Adds one completed read; returns the expected profile slots it missed.
    fn add(
        &mut self,
        kind: &DocType,
        expected: &[Expected],
        outcome: &Outcome,
    ) -> BTreeSet<String> {
        self.documents += 1;
        self.type_right += u32::from(outcome.classification.doc_type.as_deref() == Some(kind.key));
        let mut missed = BTreeSet::new();
        for e in expected.iter().filter(|e| e.want.is_some()) {
            self.slots_expected += 1;
            if outcome.values.iter().any(|v| holds(e, v)) {
                self.slots_found += 1;
            } else {
                missed.extend(e.slot.clone());
            }
        }
        self.slots_made += outcome.values.len() as u32;
        self.slots_right += outcome
            .values
            .iter()
            .filter(|v| expected.iter().any(|e| holds(e, v)))
            .count() as u32;
        // A value printed twice (text layer and OCR rows) is one expected fact.
        let mut groups: BTreeMap<(&str, &str), bool> = BTreeMap::new();
        for e in expected {
            let read = outcome.facts.iter().find(|f| reads(e, f));
            *groups.entry((&e.label, &e.value)).or_default() |= read.is_some();
            let Some(f) = read else { continue };
            if let Some(owner) = &e.owner {
                self.owners_judged += 1;
                self.owners_right += u32::from(f.owner == *owner);
            }
            if let Some(period) = &e.period {
                self.periods_judged += 1;
                self.periods_right += u32::from(f.period.as_deref() == Some(period.as_str()));
            }
        }
        self.facts_expected += groups.len() as u32;
        self.facts_found += groups.values().filter(|found| **found).count() as u32;
        let required_missing = kind.tier == Tier::Eager
            && kind
                .slots
                .iter()
                .filter(|s| s.required)
                .any(|s| !outcome.values.iter().any(|v| v.slot == s.key));
        self.false_completions += u32::from(required_missing);
        self.checks += outcome.values.iter().filter(|v| v.check).count() as u32;
        self.uninterpreted += outcome.uninterpreted as u32;
        self.reader_requests += outcome.reader_requests;
        self.typesafe_requests += outcome.typesafe_requests;
        missed
    }

    fn line(&self, family: &str, failed: u32) -> String {
        format!(
            "family {family}: {} read, {failed} failed · type {} · slot recall {} · slot precision {} · fact recall {} · owner accuracy {} · period accuracy {} · false completion {} · checks per document {} · not interpreted per document {} · reader requests per document {} · TypeSafe requests per document {}",
            self.documents,
            ratio(self.type_right, self.documents),
            ratio(self.slots_found, self.slots_expected),
            ratio(self.slots_right, self.slots_made),
            ratio(self.facts_found, self.facts_expected),
            ratio(self.owners_right, self.owners_judged),
            ratio(self.periods_right, self.periods_judged),
            ratio(self.false_completions, self.documents),
            per_document(self.checks, self.documents),
            per_document(self.uninterpreted, self.documents),
            per_document(self.reader_requests, self.documents),
            per_document(self.typesafe_requests, self.documents),
        )
    }
}

/// Offline check of the scorer, so a paid run cannot fail on it: a read that
/// returns exactly the expectations scores full marks, an empty read none.
#[test]
fn the_scorer_gives_a_perfect_read_full_marks_and_an_empty_read_none() {
    let specs: Value = serde_json::from_str(
        &std::fs::read_to_string(format!("{FIXTURES}/expectations.json")).unwrap(),
    )
    .unwrap();
    for (name, spec) in specs.as_object().unwrap() {
        let text = std::fs::read_to_string(format!("{FIXTURES}/{name}")).unwrap();
        let kind = doc_type(spec["doc_type"].as_str().unwrap()).expect("registry type");
        let source = segments(&text);
        let segments: Vec<SourceSegment<'_>> = source
            .iter()
            .map(|s| SourceSegment {
                id: &s.segment_id,
                text: &s.text,
            })
            .collect();
        let expected = expectations(spec, &segments, kind);
        let classification = Classification {
            family: kind.family.into(),
            family_confidence: 1.,
            doc_type: Some(kind.key.into()),
            type_confidence: Some(1.),
            subject: None,
            subject_confidence: 0.,
            subject_name: None,
            readable: 1.,
            mixed: 0.,
            usage: Default::default(),
        };
        let values = expected
            .iter()
            .filter_map(|e| {
                let content = match e.want.as_ref()? {
                    Want::Key(key) => SlotContent::Category { key: key.clone() },
                    want => {
                        let value = match want {
                            Want::Value(v) => (**v).clone(),
                            _ => CandidateValue::Text(e.value.clone()),
                        };
                        let (segment_id, start, end) = e.at.clone();
                        SlotContent::Candidate(Box::new(me_core::Candidate {
                            id: String::new(),
                            kind: me_core::CandidateKind::LabelValue,
                            text: e.value.clone(),
                            value,
                            segment_id,
                            start,
                            end,
                            line: String::new(),
                            label: None,
                            checksum: false,
                        }))
                    }
                };
                Some(SlotValue {
                    slot: e.slot.clone()?,
                    content,
                    period: e.payment_period,
                    confidence: 1.,
                    source: me_core::ConfidenceSource::Typesafe,
                    value_checked: false,
                    check: false,
                })
            })
            .collect();
        let facts = expected
            .iter()
            .map(|e| ReadFact {
                label: e.label.clone(),
                value: e.value.clone(),
                segment_id: e.at.0.clone(),
                start: e.at.1,
                end: e.at.2,
                context: String::new(),
                owner: e.owner.clone().unwrap_or_else(|| "unclear".into()),
                owner_entity: None,
                owner_name: None,
                period: e.period.clone(),
                slot: e.slot.clone(),
                state: me_core::FactState::Verified,
                confidence: Some(1.),
            })
            .collect();
        let perfect = Outcome {
            classification,
            values,
            facts,
            uninterpreted: 0,
            reader_requests: 1,
            typesafe_requests: 3,
        };
        let mut t = Tally::default();
        assert!(t.add(kind, &expected, &perfect).is_empty(), "{name}");
        assert_eq!(t.type_right, 1, "{name}");
        assert_eq!(t.slots_found, t.slots_expected, "{name}");
        assert_eq!(
            (t.slots_right, t.slots_made),
            (t.slots_found, t.slots_expected)
        );
        assert_eq!(t.facts_found, t.facts_expected, "{name}");
        assert_eq!(t.owners_right, t.owners_judged, "{name}");
        assert_eq!(t.periods_right, t.periods_judged, "{name}");
        assert_eq!(t.false_completions, 0, "{name}");
        assert_eq!(kind.tier == Tier::Eager, t.slots_expected > 0, "{name}");

        let empty = Outcome {
            values: Vec::new(),
            facts: Vec::new(),
            ..perfect
        };
        let mut t = Tally::default();
        t.add(kind, &expected, &empty);
        assert_eq!((t.slots_found, t.slots_made, t.facts_found), (0, 0, 0));
        let required = kind.slots.iter().any(|s| s.required);
        assert_eq!(t.false_completions, u32::from(required), "{name}");
    }
}

#[test]
#[ignore = "Live: the signed-in ME_CODEX_TEST_HOME reader and TypeSafe over the synthetic corpus"]
fn live_eval_reads_the_synthetic_corpus() {
    let home = PathBuf::from(
        std::env::var_os("ME_CODEX_TEST_HOME").expect("Explicit test Codex home required"),
    );
    let mut typesafe = Counted {
        inner: TypeSafe::configured().expect("TypeSafe configuration"),
        requests: 0,
    };
    let anchors = anchors();
    let specs: Value = serde_json::from_str(
        &std::fs::read_to_string(format!("{FIXTURES}/expectations.json")).unwrap(),
    )
    .unwrap();
    let mut families: BTreeMap<&str, (Tally, u32)> = BTreeMap::new();
    let mut failures = Vec::new();
    for (n, (name, spec)) in specs.as_object().unwrap().iter().enumerate() {
        let text = std::fs::read_to_string(format!("{FIXTURES}/{name}")).unwrap();
        let kind = doc_type(spec["doc_type"].as_str().unwrap()).expect("registry type");
        // A neutral title: the fixture's file name would give the type away.
        let title = format!("scan_{:02}.pdf", n + 1);
        let started = std::time::Instant::now();
        let result = read(&home, &title, &text, &anchors, &mut typesafe);
        let entry = families.entry(kind.family).or_default();
        match result {
            Ok(outcome) => {
                let source = segments(&text);
                let segments: Vec<SourceSegment<'_>> = source
                    .iter()
                    .map(|s| SourceSegment {
                        id: &s.segment_id,
                        text: &s.text,
                    })
                    .collect();
                let expected = expectations(spec, &segments, kind);
                let missed = entry.0.add(kind, &expected, &outcome);
                let c = &outcome.classification;
                println!(
                    "{name}: {}/{} (expected {}) · {} facts · {} profile values · missed slots {missed:?} · {} checks · {} not interpreted · reader {} · TypeSafe {} · {} s",
                    c.family,
                    c.doc_type.as_deref().unwrap_or("-"),
                    kind.key,
                    outcome.facts.len(),
                    outcome.values.len(),
                    outcome.values.iter().filter(|v| v.check).count(),
                    outcome.uninterpreted,
                    outcome.reader_requests,
                    outcome.typesafe_requests,
                    started.elapsed().as_secs(),
                );
            }
            Err(message) => {
                entry.1 += 1;
                println!("{name}: did not complete: {message}");
                failures.push(name.clone());
            }
        }
    }
    let mut total = Tally::default();
    let mut failed = 0;
    for (family, (tally, family_failed)) in &families {
        println!("{}", tally.line(family, *family_failed));
        failed += family_failed;
        for (sum, part) in [
            (&mut total.documents, tally.documents),
            (&mut total.type_right, tally.type_right),
            (&mut total.slots_expected, tally.slots_expected),
            (&mut total.slots_found, tally.slots_found),
            (&mut total.slots_made, tally.slots_made),
            (&mut total.slots_right, tally.slots_right),
            (&mut total.facts_expected, tally.facts_expected),
            (&mut total.facts_found, tally.facts_found),
            (&mut total.owners_judged, tally.owners_judged),
            (&mut total.owners_right, tally.owners_right),
            (&mut total.periods_judged, tally.periods_judged),
            (&mut total.periods_right, tally.periods_right),
            (&mut total.false_completions, tally.false_completions),
            (&mut total.checks, tally.checks),
            (&mut total.uninterpreted, tally.uninterpreted),
            (&mut total.reader_requests, tally.reader_requests),
            (&mut total.typesafe_requests, tally.typesafe_requests),
        ] {
            *sum += part;
        }
    }
    println!("{}", total.line("all", failed));
    // A baseline, not a gate: only completion is asserted.
    assert!(
        failures.is_empty(),
        "{} document(s) did not complete: {failures:?}",
        failures.len()
    );
}
