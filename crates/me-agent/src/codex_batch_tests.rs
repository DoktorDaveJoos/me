use super::*;

fn long_input() -> ExtractionInput {
    ExtractionInput {
        run_id: "synthetic-run".into(),
        source_id: "synthetic-source".into(),
        title: "Synthetic".into(),
        segments: (1..=26)
            .map(|i| me_core::SourceText {
                segment_id: format!("segment-{i}"),
                ordinal: i,
                text: format!("Erika Beispiel\nSteuer-ID: {i:011}\n{}", "ü".repeat(700)),
            })
            .collect(),
    }
}
#[test]
fn unicode_sections_keep_all_text_ids_and_order_with_bounded_requests() {
    let input = long_input();
    let batches = extraction_batches(&input).unwrap();
    assert!(batches.len() >= 3);
    for batch in &batches {
        assert!(batch.segments.iter().map(|s| s.text.len()).sum::<usize>() <= BATCH_TEXT_BYTES);
    }
    let mut actual: Vec<_> = batches
        .iter()
        .flat_map(|b| &b.segments)
        .map(|s| (&s.segment_id, s.ordinal, &s.text))
        .collect();
    let expected: Vec<_> = input
        .segments
        .iter()
        .map(|s| (&s.segment_id, s.ordinal, &s.text))
        .collect();
    actual.dedup();
    assert_eq!(actual, expected);
    let mut invalid = input.clone();
    invalid.segments[0].text = "a".repeat(BATCH_TEXT_BYTES + 1);
    assert!(extraction_batches(&invalid).is_err());
}
#[test]
fn reports_real_section_and_model_phase_boundaries() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("fake-codex");
    tests::fake(&bin, "batches");
    let input = long_input();
    let mut stages = Vec::new();
    let mut sections = Vec::new();
    extract_with_binary(
        &temp.path().join("home"),
        &input,
        Arc::new(AtomicBool::new(false)),
        &mut |event| match event {
            Progress::Stage(stage) => stages.push(stage),
            Progress::Units { current, total } => sections.push((current, total)),
            _ => {}
        },
        &bin,
    )
    .unwrap();
    assert_eq!(sections.len(), extraction_batches(&input).unwrap().len());
    assert_eq!(sections.last().unwrap().0, sections.last().unwrap().1);
    for stage in [
        me_core::ImportStage::Interpreting,
        me_core::ImportStage::Context,
        me_core::ImportStage::Extracting,
        me_core::ImportStage::Verifying,
    ] {
        assert!(stages.contains(&stage));
    }
    assert_eq!(stages.last(), Some(&me_core::ImportStage::Verifying));
}
#[test]
fn all_sections_are_requested_and_late_failure_discards_earlier_results() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("fake-codex");
    let input = long_input();
    tests::fake(&bin, "batches");
    let output = extract_with_binary(
        &temp.path().join("home"),
        &input,
        Arc::new(AtomicBool::new(false)),
        &mut |_| {},
        &bin,
    )
    .unwrap();
    assert_eq!(output.facts.len(), 26);
    assert_eq!(output.facts.last().unwrap().segment_id, "segment-26");
    tests::fake(&bin, "late_failure");
    let result = extract_with_binary(
        &temp.path().join("home"),
        &input,
        Arc::new(AtomicBool::new(false)),
        &mut |_| {},
        &bin,
    );
    assert!(result.unwrap_err().contains("Section 2 of"));
}
#[test]
fn cancelling_between_sections_does_not_return_partial_results() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("fake-codex");
    tests::fake(&bin, "batches");
    let cancel = Arc::new(AtomicBool::new(false));
    let result = extract_with_binary(
        &temp.path().join("home"),
        &long_input(),
        cancel.clone(),
        &mut |p| {
            if matches!(p, Progress::Message(ref s) if s.contains("Section 2")) {
                cancel.store(true, Ordering::SeqCst);
            }
        },
        &bin,
    );
    assert!(result.unwrap_err().contains("stopped"));
}

#[derive(Default)]
struct MemoryCheckpoints(std::collections::HashMap<String, ExtractionOutput>);
impl Checkpoints for MemoryCheckpoints {
    fn load(&mut self, input: &ExtractionInput) -> Result<Option<ExtractionOutput>> {
        Ok(self.0.get(&me_core::extraction_fingerprint(input)).cloned())
    }
    fn save(&mut self, input: &ExtractionInput, output: &ExtractionOutput) -> Result<()> {
        self.0
            .insert(me_core::extraction_fingerprint(input), output.clone());
        Ok(())
    }
}
#[test]
fn resumes_verified_sections_after_cancel_without_resending_them() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("fake-codex");
    tests::fake(&bin, "batches");
    let input = long_input();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut cache = MemoryCheckpoints::default();
    let result = pipeline::run(
        &temp.path().join("home"),
        &input,
        cancel.clone(),
        &mut |p| {
            if matches!(p,Progress::Message(ref s) if s.contains("Section 2")) {
                cancel.store(true, Ordering::SeqCst);
            }
        },
        &bin,
        &mut cache,
    );
    assert!(result.is_err());
    assert_eq!(cache.0.len(), 1);
    let mut resumed = false;
    let output = pipeline::run(
        &temp.path().join("home"),
        &input,
        Arc::new(AtomicBool::new(false)),
        &mut |p| {
            resumed |=
                matches!(p,Progress::Message(ref s) if s.contains("restoring saved results"));
        },
        &bin,
        &mut cache,
    )
    .unwrap();
    assert!(resumed);
    assert_eq!(output.output.facts.len(), 26);
    let calls = fs::read_to_string(bin.with_extension("calls")).unwrap();
    assert_eq!(
        calls.lines().count(),
        extraction_batches(&input).unwrap().len()
    );
}
#[test]
fn context_errors_split_and_transient_errors_retry_without_losing_facts() {
    for mode in ["large", "transient"] {
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("fake-codex");
        tests::fake(&bin, mode);
        let result = pipeline::run(
            &temp.path().join("home"),
            &long_input(),
            Arc::new(AtomicBool::new(false)),
            &mut |_| {},
            &bin,
            &mut MemoryCheckpoints::default(),
        )
        .unwrap();
        assert_eq!(result.output.facts.len(), 26);
        assert_eq!(result.rejected.len(), 0);
    }
}
#[test]
fn retries_bad_evidence_and_preserves_unverified_candidates_for_review() {
    let mut input = long_input();
    input.segments.truncate(4);
    for (mode, rejected, count) in [("repair", 0, 4), ("bad_evidence", 1, 3)] {
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("fake-codex");
        tests::fake(&bin, mode);
        let mut cache = MemoryCheckpoints::default();
        let result = pipeline::run(
            &temp.path().join("home"),
            &input,
            Arc::new(AtomicBool::new(false)),
            &mut |_| {},
            &bin,
            &mut cache,
        )
        .unwrap();
        assert_eq!(result.output.facts.len(), count);
        assert_eq!(result.rejected.len(), rejected);
        assert_eq!(cache.0.len(), usize::from(rejected == 0));
        if rejected > 0 {
            assert!(!result.rejected[0].fact.value.is_empty());
            assert!(!result.rejected[0].fact.quote.is_empty());
            assert_eq!(result.rejected[0].code, "quote_not_in_segment");
        }
    }
}

#[test]
fn maximum_supported_text_is_chunked_without_dropping_segments() {
    let mut input = long_input();
    input.segments.clear();
    let mut remaining = me_core::MAX_DOCUMENT_TEXT;
    while remaining > 0 {
        let bytes = remaining.min(1600);
        let ordinal = input.segments.len() as i64 + 1;
        input.segments.push(me_core::SourceText {
            segment_id: format!("s{ordinal}"),
            ordinal,
            text: "a".repeat(bytes),
        });
        remaining -= bytes;
    }
    let batches = extraction_batches(&input).unwrap();
    assert!(batches.len() > 500);
    let mut ordinals = std::collections::HashSet::new();
    for batch in batches {
        assert!(batch.segments.iter().map(|s| s.text.len()).sum::<usize>() <= BATCH_TEXT_BYTES);
        for segment in batch.segments {
            ordinals.insert(segment.ordinal);
        }
    }
    assert_eq!(ordinals.len(), input.segments.len());
    input.segments[0].text.push('a');
    assert!(extraction_batches(&input).is_err());
}

#[test]
fn supported_value_for_one_person_does_not_hide_an_unsupported_person_claim() {
    let mut input = long_input();
    input.segments.truncate(4);
    input.segments[3].text = input.segments[0].text.clone();
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("fake-codex");
    tests::fake(&bin, "bad_subject");
    let result = pipeline::run(
        &temp.path().join("home"),
        &input,
        Arc::new(AtomicBool::new(false)),
        &mut |_| {},
        &bin,
        &mut MemoryCheckpoints::default(),
    )
    .unwrap();
    assert_eq!(result.output.facts.len(), 3);
    assert_eq!(result.rejected.len(), 1);
}

#[test]
fn independent_audit_recovers_more_than_sixteen_missing_fields() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("fake-codex");
    tests::fake(&bin, "audit_missing");
    let mut input = long_input();
    input.segments.truncate(1);
    input.segments[0].text = format!(
        "Erika Beispiel\nAugust 2026\n{}",
        (0..24)
            .map(|i| format!("Zulage {i}: {i},00 EUR"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let result = pipeline::run(
        &temp.path().join("home"),
        &input,
        Arc::new(AtomicBool::new(false)),
        &mut |_| {},
        &bin,
        &mut MemoryCheckpoints::default(),
    )
    .unwrap();
    assert_eq!(result.output.facts.len(), 24);
    assert!(result.rejected.is_empty());
}
#[test]
fn failed_completeness_audit_does_not_cache_or_report_success() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("fake-codex");
    tests::fake(&bin, "audit_failure");
    let mut cache = MemoryCheckpoints::default();
    let result = pipeline::run(
        &temp.path().join("home"),
        &long_input(),
        Arc::new(AtomicBool::new(false)),
        &mut |_| {},
        &bin,
        &mut cache,
    );
    assert!(result.is_err());
    assert!(cache.0.is_empty());
}
