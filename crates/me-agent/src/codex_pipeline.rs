use super::*;
use me_core::{ExtractedFact, ground_extraction};

pub trait Checkpoints {
    fn load(&mut self, input: &ExtractionInput) -> Result<Option<ExtractionOutput>>;
    fn save(&mut self, input: &ExtractionInput, output: &ExtractionOutput) -> Result<()>;
}
pub struct NoCheckpoints;
impl Checkpoints for NoCheckpoints {
    fn load(&mut self, _: &ExtractionInput) -> Result<Option<ExtractionOutput>> {
        Ok(None)
    }
    fn save(&mut self, _: &ExtractionInput, _: &ExtractionOutput) -> Result<()> {
        Ok(())
    }
}
pub const PIPELINE: &str = "document-v4-gpt-5.6-sol-medium-context-audit-v1";
pub struct ExtractionReport {
    pub output: ExtractionOutput,
    pub rejected: Vec<me_core::RejectedFact>,
    pub repaired: usize,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Retry {
    Never,
    Transient,
    Smaller,
    InvalidOutput,
}

pub(super) fn run(
    home: &Path,
    input: &ExtractionInput,
    cancel: Arc<AtomicBool>,
    progress: &mut impl FnMut(Progress),
    binary: &Path,
    checkpoints: &mut impl Checkpoints,
) -> Result<ExtractionReport> {
    let batches = extraction_batches(input)?;
    let scratch = tempfile::Builder::new()
        .prefix("me-inbox-")
        .tempdir()
        .map_err(failed)?;
    let mut runner = Runner {
        home,
        binary,
        scratch: scratch.path(),
        cancel,
        progress,
        checkpoints,
        session: None,
    };
    record(
        "codex.document",
        &[
            F::Count("batches", batches.len() as u64),
            F::Count(
                "text_bytes",
                input.segments.iter().map(|s| s.text.len() as u64).sum(),
            ),
        ],
    );
    let mut report = ExtractionReport {
        output: ExtractionOutput { facts: Vec::new() },
        rejected: Vec::new(),
        repaired: 0,
    };
    for (index, batch) in batches.iter().enumerate() {
        (runner.progress)(Progress::Units {
            current: (index + 1) as u32,
            total: batches.len() as u32,
        });
        let label = format!("Section {} of {}", index + 1, batches.len());
        (runner.progress)(Progress::Message(format!("Reading {label}…")));
        let result = runner
            .section(batch, &label, 0)
            .map_err(|e| format!("{label}: {e}"))?;
        merge(&mut report.output, result.output);
        merge_questions(&mut report.rejected, result.rejected, &report.output);
        report.repaired += result.repaired;
        if report.output.facts.len() + report.rejected.len() > me_core::MAX_EXTRACTION_FACTS {
            return Err("Too many different details. Split the document by person.".into());
        }
    }
    Ok(report)
}

struct Runner<'a, P, C> {
    home: &'a Path,
    binary: &'a Path,
    scratch: &'a Path,
    cancel: Arc<AtomicBool>,
    progress: &'a mut P,
    checkpoints: &'a mut C,
    session: Option<Session>,
}
fn merge(target: &mut ExtractionOutput, output: ExtractionOutput) {
    for fact in output.facts {
        me_core::merge_extracted_fact(&mut target.facts, fact);
    }
}
fn merge_questions(
    target: &mut Vec<me_core::RejectedFact>,
    incoming: Vec<me_core::RejectedFact>,
    good: &ExtractionOutput,
) {
    for question in incoming {
        if !target
            .iter()
            .any(|old| same_value(&old.fact, &question.fact))
        {
            target.push(question);
        }
    }
    target.retain(|question| {
        !good
            .facts
            .iter()
            .any(|fact| same_value(fact, &question.fact))
    });
}
fn same_value(a: &ExtractedFact, b: &ExtractedFact) -> bool {
    a.property == b.property
        && (me_core::document_field_label(&a.property).is_none()
            || (a
                .context_quote
                .split_whitespace()
                .eq(b.context_quote.split_whitespace())
                && (!a.context_quote.is_empty() || a.segment_id == b.segment_id)))
        && a.subject_quote
            .split_whitespace()
            .eq(b.subject_quote.split_whitespace())
        && match (
            me_core::normalize_fact(&a.property, &a.value),
            me_core::normalize_fact(&b.property, &b.value),
        ) {
            (Ok(a), Ok(b)) => a == b,
            _ => a.value == b.value,
        }
}
impl<P: FnMut(Progress), C: Checkpoints> Runner<'_, P, C> {
    fn cancelled(&self) -> Result<()> {
        if self.cancel.load(Ordering::SeqCst) {
            Err("Analysis stopped. Completed sections are saved for the next attempt.".into())
        } else {
            Ok(())
        }
    }
    fn connect(&mut self) -> Result<()> {
        if self.session.is_some() {
            return Ok(());
        }
        let mut session = Session::start(self.home, self.scratch, self.cancel.clone(), self.binary)
            .inspect_err(|message| {
                (self.progress)(Progress::SetupRequired(SetupIssue::Connection(
                    message.clone(),
                )));
            })?;
        session.deadline = Instant::now() + Duration::from_secs(30);
        if let Err(issue) =
            initialize(&mut session).and_then(|()| verify_configuration(&mut session))
        {
            let message = issue.message();
            (self.progress)(Progress::SetupRequired(issue));
            return Err(message);
        }
        self.session = Some(session);
        Ok(())
    }
    fn section(
        &mut self,
        input: &ExtractionInput,
        label: &str,
        depth: u8,
    ) -> Result<ExtractionReport> {
        self.cancelled()?;
        if let Some(cached) = self.checkpoints.load(input)? {
            let checked = ground_extraction(input, cached);
            if checked.rejected.is_empty() {
                record(
                    "codex.batch.resumed",
                    &[F::Count("segments", input.segments.len() as u64)],
                );
                (self.progress)(Progress::Message(format!(
                    "{label}: restoring saved results…"
                )));
                return Ok(ExtractionReport {
                    output: checked.output,
                    rejected: Vec::new(),
                    repaired: checked.repaired,
                });
            }
        }
        let mut good = ExtractionOutput { facts: Vec::new() };
        let mut rejected: Vec<me_core::RejectedFact> = Vec::new();
        let mut repaired = 0;
        let mut correction = false;
        for attempt in 1..=2 {
            self.cancelled()?;
            self.connect()?;
            let session = self.session.as_mut().ok_or(FAILURE)?;
            session.retry = Retry::Never;
            session.deadline = Instant::now() + Duration::from_secs(240);
            record(
                "codex.batch.started",
                &[
                    F::Count("attempt", attempt),
                    F::Integer(
                        "first_segment",
                        input.segments.first().map(|s| s.ordinal).unwrap_or(0),
                    ),
                    F::Count("split_depth", depth as u64),
                    F::Flag("correction", correction),
                ],
            );
            let started = Instant::now();
            let result = extract_batch(
                session,
                self.scratch,
                input,
                self.progress,
                correction,
                &rejected,
            );
            record(
                "codex.batch.finished",
                &[
                    F::Count("attempt", attempt),
                    F::Integer(
                        "first_segment",
                        input.segments.first().map(|s| s.ordinal).unwrap_or(0),
                    ),
                    F::Flag("ok", result.is_ok()),
                    F::Count("duration_ms", started.elapsed().as_millis() as u64),
                ],
            );
            let retry = session.retry;
            self.cancelled()?;
            match result {
                Ok(output) if output.facts.len() < MAX_BATCH_FACTS * 2 => {
                    let checked = ground_extraction(input, output);
                    repaired += checked.repaired;
                    merge(&mut good, checked.output);
                    rejected.extend(checked.rejected);
                    rejected
                        .retain(|bad| !good.facts.iter().any(|fact| same_value(fact, &bad.fact)));
                    let mut distinct = Vec::<me_core::RejectedFact>::new();
                    for bad in rejected.drain(..) {
                        if !distinct.iter().any(|old| same_value(&old.fact, &bad.fact)) {
                            distinct.push(bad);
                        }
                    }
                    rejected = distinct;
                    for bad in &rejected {
                        record("codex.evidence.rejected", &[F::Label("code", bad.code)]);
                    }
                    if rejected.is_empty() {
                        break;
                    }
                    if attempt == 1 {
                        correction = true;
                        (self.progress)(Progress::Message(format!(
                            "{label}: checking sources again…"
                        )));
                    }
                }
                result => {
                    let retry = if result.is_ok() {
                        Retry::Smaller
                    } else {
                        retry
                    };
                    if input.segments.len() > 1
                        && depth < 3
                        && (retry == Retry::Smaller
                            || (retry == Retry::InvalidOutput && attempt == 2))
                    {
                        record(
                            "codex.batch.split",
                            &[
                                F::Count("segments", input.segments.len() as u64),
                                F::Count("depth", depth as u64 + 1),
                            ],
                        );
                        (self.progress)(Progress::Message(format!(
                            "{label}: splitting into smaller sections…"
                        )));
                        let middle = input.segments.len() / 2;
                        let mut combined = ExtractionReport {
                            output: good,
                            rejected,
                            repaired,
                        };
                        for (index, segments) in [
                            input.segments[..middle].to_vec(),
                            input.segments[middle..].to_vec(),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            let child = input.with_segments(segments);
                            let part =
                                self.section(&child, &format!("{label}.{}", index + 1), depth + 1)?;
                            merge(&mut combined.output, part.output);
                            merge_questions(
                                &mut combined.rejected,
                                part.rejected,
                                &combined.output,
                            );
                            combined.repaired += part.repaired;
                        }
                        if combined.rejected.is_empty() {
                            self.checkpoints.save(input, &combined.output)?;
                        }
                        return Ok(combined);
                    }
                    if attempt == 1 && [Retry::Transient, Retry::InvalidOutput].contains(&retry) {
                        self.session = None;
                        record("codex.batch.retry", &[]);
                        (self.progress)(Progress::Message(format!(
                            "{label}: retrying after a temporary error…"
                        )));
                        for _ in 0..20 {
                            self.cancelled()?;
                            thread::sleep(Duration::from_millis(50));
                        }
                        continue;
                    }
                    return Err(result
                        .err()
                        .unwrap_or_else(|| "Too many suggestions for one section.".into()));
                }
            }
        }
        if rejected.is_empty() {
            self.checkpoints.save(input, &good)?;
        }
        record(
            "codex.evidence.checked",
            &[
                F::Count("repaired", repaired as u64),
                F::Count("rejected", rejected.len() as u64),
            ],
        );
        Ok(ExtractionReport {
            output: good,
            rejected,
            repaired,
        })
    }
}
