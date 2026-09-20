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
            Progress::Step {
                stage: me_core::ImportStage::Verifying,
                current,
                total,
            } => sections.push((current, total)),
            _ => {}
        },
        &bin,
    )
    .unwrap();
    assert!(sections.len() > extraction_batches(&input).unwrap().len());
    assert_eq!(sections[0].0, 0);
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
struct MemoryCheckpoints(
    std::collections::HashMap<String, ExtractionOutput>,
    std::collections::HashMap<String, Value>,
);
impl Checkpoints for MemoryCheckpoints {
    fn load_step(&mut self, input: &ExtractionInput, step: &str) -> Result<Option<Value>> {
        Ok(self
            .1
            .get(&format!(
                "{}:{step}",
                me_core::extraction_fingerprint(input)
            ))
            .cloned())
    }
    fn save_step(&mut self, input: &ExtractionInput, step: &str, output: &Value) -> Result<()> {
        self.1.insert(
            format!("{}:{step}", me_core::extraction_fingerprint(input)),
            output.clone(),
        );
        Ok(())
    }
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
                matches!(p,Progress::Message(ref s) if s.contains("Restoring saved results"));
        },
        &bin,
        &mut cache,
    )
    .unwrap();
    assert!(resumed);
    assert_eq!(output.output.facts.len(), 26);
    let calls = fs::read_to_string(bin.with_extension("calls")).unwrap();
    assert_eq!(calls.lines().count(), cache.0.len());
}
#[test]
fn small_sections_avoid_large_requests_and_transient_failure_never_auto_retries() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("fake-codex");
    tests::fake(&bin, "large");
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
    tests::fake(&bin, "transient");
    let mut failures = Vec::new();
    let mut cache = MemoryCheckpoints::default();
    let result = pipeline::run(
        &temp.path().join("home"),
        &long_input(),
        Arc::new(AtomicBool::new(false)),
        &mut |event| {
            if let Progress::Failure(f) = event {
                failures.push(f);
            }
        },
        &bin,
        &mut cache,
    );
    assert!(result.is_err());
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].kind, me_core::ImportErrorKind::Connection);
    // Only explicit resume is allowed to make another request.
    let result = pipeline::run(
        &temp.path().join("home"),
        &long_input(),
        Arc::new(AtomicBool::new(false)),
        &mut |_| {},
        &bin,
        &mut cache,
    )
    .unwrap();
    assert_eq!(result.output.facts.len(), 26);
}
#[test]
fn one_audit_repairs_evidence_or_preserves_unverified_candidates_for_review() {
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
        assert_eq!(cache.0.len(), if rejected == 0 { 3 } else { 2 });
        if rejected > 0 {
            assert!(!result.rejected[0].fact.value.is_empty());
            assert!(!result.rejected[0].fact.quote.is_empty());
            assert_eq!(result.rejected[0].code, "quote_not_in_segment");
        }
    }
}

#[test]
fn unknown_ownership_becomes_a_question_without_a_paid_audit_or_repeated_calls_on_resume() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("fake-codex");
    tests::fake(&bin, "unknown_subject");
    let mut input = long_input();
    input.segments.truncate(1);
    input.segments[0].text = "Steuer-ID: 01234567890".into();
    let mut cache = MemoryCheckpoints::default();
    let mut calls = Vec::new();
    for attempt in 0..2 {
        input.run_id = format!("attempt-{attempt}");
        let result = pipeline::run(
            &temp.path().join("home"),
            &input,
            Arc::new(AtomicBool::new(false)),
            &mut |event| {
                if let Progress::Request { provider } = event {
                    calls.push(provider)
                }
            },
            &bin,
            &mut cache,
        )
        .unwrap();
        assert!(result.output.facts.is_empty());
        assert_eq!(result.rejected.len(), 1);
        assert_eq!(result.rejected[0].code, "subject_unknown");
        assert_eq!(result.rejected[0].fact.value, "01234567890");
        assert!(result.rejected[0].fact.subject_quote.is_empty());
    }
    assert_eq!(
        calls
            .iter()
            .filter(|p| **p == me_core::ImportProvider::OpenAi)
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|p| **p == me_core::ImportProvider::TypeSafe)
            .count(),
        2
    );
    assert_eq!(
        fs::read_to_string(bin.with_extension("requests")).unwrap(),
        "extract\n"
    );
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
    let result = pipeline::run_with_decisions(
        &temp.path().join("home"),
        &input,
        Arc::new(AtomicBool::new(false)),
        &mut |_| {},
        &bin,
        &mut MemoryCheckpoints::default(),
        &mut AuditDecisions::default(),
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
    let result = pipeline::run_with_decisions(
        &temp.path().join("home"),
        &long_input(),
        Arc::new(AtomicBool::new(false)),
        &mut |_| {},
        &bin,
        &mut cache,
        &mut AuditDecisions::default(),
    );
    assert!(result.is_err());
    assert!(cache.0.is_empty());
}

#[derive(Default)]
struct AuditDecisions {
    calls: usize,
}
impl crate::typesafe::Decisions for AuditDecisions {
    fn evaluate(
        &mut self,
        state: Value,
        questions: Value,
        cancel: &AtomicBool,
    ) -> crate::typesafe::Result<crate::typesafe::DecisionResponse> {
        self.calls += 1;
        let mut response = crate::typesafe::FakeDecisions.evaluate(state, questions, cancel)?;
        if let Some(missing) = response.answers.get_mut("missing") {
            missing["noul"] = json!(0.8);
        }
        Ok(response)
    }
}
#[test]
fn failed_audit_resumes_only_missing_step_with_new_attempt_id() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("fake-codex");
    tests::fake(&bin, "audit_failure");
    let mut input = long_input();
    input.segments.truncate(1);
    let mut cache = MemoryCheckpoints::default();
    let mut decisions = AuditDecisions::default();
    let run =
        |input: &ExtractionInput, cache: &mut MemoryCheckpoints, decisions: &mut AuditDecisions| {
            pipeline::run_with_decisions(
                &temp.path().join("home"),
                input,
                Arc::new(AtomicBool::new(false)),
                &mut |_| {},
                &bin,
                cache,
                decisions,
            )
        };
    assert!(run(&input, &mut cache, &mut decisions).is_err());
    assert_eq!(decisions.calls, 2);
    assert_eq!(cache.1.len(), 3);
    assert!(cache.0.is_empty());
    tests::fake(&bin, "ok");
    input.run_id = "new-attempt-after-restart".into();
    assert!(run(&input, &mut cache, &mut decisions).is_ok());
    assert_eq!(
        decisions.calls, 2,
        "Resuming must reuse both TypeSafe decisions"
    );
    let requests = fs::read_to_string(bin.with_extension("requests")).unwrap();
    assert_eq!(
        requests.lines().collect::<Vec<_>>(),
        vec!["extract", "audit", "audit"]
    );
    // Fully verified recovery needs no provider at all.
    assert!(run(&input, &mut cache, &mut decisions).is_ok());
    assert_eq!(
        fs::read_to_string(bin.with_extension("requests")).unwrap(),
        requests
    );
}
#[test]
fn provider_error_objects_are_classified_without_leaking_messages() {
    let failure = provider_failure(
        &json!({"codexErrorInfo":{"httpConnectionFailed":{"httpStatusCode":429}},"message":"private"}),
    );
    assert_eq!(failure.kind, me_core::ImportErrorKind::RateLimit);
    assert!(!failure.message.contains("private"));
}

#[test]
fn cancellation_after_a_decision_keeps_the_paid_answer_for_resume() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("fake-codex");
    tests::fake(&bin, "ok");
    let mut input = long_input();
    input.segments.truncate(1);
    let mut cache = MemoryCheckpoints::default();
    let mut decisions = AuditDecisions::default();
    let cancel = Arc::new(AtomicBool::new(false));
    let result = pipeline::run_with_decisions(
        &temp.path().join("home"),
        &input,
        cancel.clone(),
        &mut |event| {
            if matches!(event, Progress::Usage { .. }) {
                cancel.store(true, Ordering::SeqCst);
            }
        },
        &bin,
        &mut cache,
        &mut decisions,
    );
    assert!(result.is_err());
    assert_eq!(cache.1.len(), 1);
    assert_eq!(decisions.calls, 1);
    input.run_id = "resume-after-paid-answer".into();
    pipeline::run_with_decisions(
        &temp.path().join("home"),
        &input,
        Arc::new(AtomicBool::new(false)),
        &mut |_| {},
        &bin,
        &mut cache,
        &mut decisions,
    )
    .unwrap();
    assert_eq!(
        decisions.calls, 2,
        "Only verification needs a new TypeSafe request"
    );
}
