use super::*;
use crate::typesafe::{self, Decisions};
use me_core::{
    ExtractedFact, ImportErrorKind as Kind, ImportFailure, ImportProvider as Provider,
    ImportStage as Stage, ground_extraction,
};

pub trait Checkpoints {
    fn load(&mut self, input: &ExtractionInput) -> Result<Option<ExtractionOutput>>;
    fn save(&mut self, input: &ExtractionInput, output: &ExtractionOutput) -> Result<()>;
    fn load_step(&mut self, _: &ExtractionInput, _: &str) -> Result<Option<Value>> {
        Ok(None)
    }
    fn save_step(&mut self, _: &ExtractionInput, _: &str, _: &Value) -> Result<()> {
        Ok(())
    }
    fn reserve(
        &mut self,
        _: &ExtractionInput,
        _: Provider,
    ) -> std::result::Result<String, ImportFailure> {
        Ok(uuid::Uuid::new_v4().to_string())
    }
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
// Candidate capture can become more inclusive without invalidating paid source
// checkpoints. Existing results remain reusable; unfinished steps use the new guidance.
pub const PIPELINE: &str = "document-v5-typesafe-jev-small-sections-sol-v1";
pub const LEGACY_PIPELINE: &str = "document-v4-gpt-5.6-sol-medium-context-audit-v1";
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
    #[cfg(test)]
    let mut decisions = typesafe::FakeDecisions;
    #[cfg(not(test))]
    let mut decisions = typesafe::LazyDecisions::default();
    run_with_decisions(
        home,
        input,
        cancel,
        progress,
        binary,
        checkpoints,
        &mut decisions,
    )
}

pub(super) fn run_with_decisions(
    home: &Path,
    input: &ExtractionInput,
    cancel: Arc<AtomicBool>,
    progress: &mut impl FnMut(Progress),
    binary: &Path,
    checkpoints: &mut impl Checkpoints,
    decisions: &mut impl Decisions,
) -> Result<ExtractionReport> {
    // Reuse whole previously verified sections where possible. Otherwise plan
    // small units up front: output failures never recursively multiply calls.
    let mut batches = Vec::new();
    for parent in extraction_batches(input)? {
        if let Some(saved) = checkpoints.load(&parent)?
            && ground_extraction(&parent, saved).rejected.is_empty()
        {
            batches.push(parent);
            continue;
        }
        let mut part = parent.with_segments(Vec::new());
        let mut bytes = 0;
        for segment in &parent.segments {
            if bytes + segment.text.len() > 3_200 && !part.segments.is_empty() {
                let overlap = part
                    .segments
                    .last()
                    .filter(|s| s.text.len() + segment.text.len() <= 3_200)
                    .cloned();
                batches.push(part);
                part = parent.with_segments(overlap.into_iter().collect());
                bytes = part.segments.iter().map(|s| s.text.len()).sum();
            }
            bytes += segment.text.len();
            part.segments.push(segment.clone());
        }
        if !part.segments.is_empty() {
            batches.push(part);
        }
    }
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
        decisions,
        session: None,
        calls: 0,
    };
    let total = batches.len() as u32;
    for stage in [
        Stage::Interpreting,
        Stage::Context,
        Stage::Extracting,
        Stage::Verifying,
    ] {
        (runner.progress)(Progress::Step {
            stage,
            current: 0,
            total,
        });
    }
    let mut report = ExtractionReport {
        output: ExtractionOutput { facts: Vec::new() },
        rejected: Vec::new(),
        repaired: 0,
    };
    for (index, batch) in batches.iter().enumerate() {
        let completed = index as u32;
        (runner.progress)(Progress::Message(format!(
            "Section {} of {}",
            index + 1,
            total
        )));
        let result = runner
            .section(batch, completed, total)
            .map_err(|e| format!("Section {} of {total}: {e}", index + 1))?;
        merge(&mut report.output, result.output);
        merge_questions(&mut report.rejected, result.rejected, &report.output);
        report.repaired += result.repaired;
        if report.output.facts.len() + report.rejected.len() > me_core::MAX_EXTRACTION_FACTS {
            return Err("Too many different details. Split the document by person.".into());
        }
    }
    Ok(report)
}
struct Runner<'a, P, C, D> {
    home: &'a Path,
    binary: &'a Path,
    scratch: &'a Path,
    cancel: Arc<AtomicBool>,
    progress: &'a mut P,
    checkpoints: &'a mut C,
    decisions: &'a mut D,
    session: Option<Session>,
    calls: u32,
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
impl<P: FnMut(Progress), C: Checkpoints, D: Decisions> Runner<'_, P, C, D> {
    fn fail<T>(&mut self, failure: ImportFailure) -> Result<T> {
        let message = failure.message.clone();
        (self.progress)(Progress::Failure(failure));
        Err(message)
    }
    fn cancelled(&self) -> Result<()> {
        if self.cancel.load(Ordering::SeqCst) {
            Err("Analysis stopped. Saved steps are kept.".into())
        } else {
            Ok(())
        }
    }
    fn start(&mut self, stage: Stage, current: u32, total: u32) {
        (self.progress)(Progress::Stage(stage));
        (self.progress)(Progress::Step {
            stage,
            current,
            total,
        });
    }
    fn done(&mut self, stage: Stage, current: u32, total: u32) {
        (self.progress)(Progress::Step {
            stage,
            current: current + 1,
            total,
        });
    }
    fn reserve(&mut self, input: &ExtractionInput, provider: Provider) -> Result<String> {
        self.cancelled()?;
        // Also bound non-vault/library callers. The vault has stricter durable
        // per-provider limits shared by every resume and reanalysis.
        if self.calls >= 96 {
            return self.fail(ImportFailure::new(
                Provider::Local,
                Kind::Budget,
                "This attempt reached its 96-call safety limit. Resume to reuse saved steps.",
            ));
        }
        match self.checkpoints.reserve(input, provider) {
            Ok(id) => {
                self.calls += 1;
                (self.progress)(Progress::Request { provider });
                Ok(id)
            }
            Err(failure) => self.fail(failure),
        }
    }
    fn cached(
        &mut self,
        input: &ExtractionInput,
        key: &str,
        state: &Value,
    ) -> Result<Option<Value>> {
        Ok(self
            .checkpoints
            .load_step(input, key)?
            .filter(|v| v["fingerprint"] == json!(fingerprint(state)))
            .map(|v| v["value"].clone()))
    }
    fn save(
        &mut self,
        input: &ExtractionInput,
        key: &str,
        state: &Value,
        value: &Value,
    ) -> Result<()> {
        self.checkpoints.save_step(
            input,
            key,
            &json!({"fingerprint":fingerprint(state),"value":value}),
        )
    }
    fn decision(
        &mut self,
        input: &ExtractionInput,
        key: &str,
        state: Value,
        questions: Value,
    ) -> Result<Value> {
        if let Some(value) = self.cached(input, key, &state)? {
            if let Err(failure) = typesafe::validate_answers(&questions, &value) {
                return self.fail(failure);
            }
            return Ok(value);
        }
        let call = self.reserve(input, Provider::TypeSafe)?;
        match self
            .decisions
            .evaluate(state.clone(), questions, &self.cancel)
        {
            Ok(response) => {
                if let (Some(input_tokens), Some(output_tokens)) =
                    (response.input_tokens, response.output_tokens)
                {
                    (self.progress)(Progress::Usage {
                        call,
                        input_tokens,
                        output_tokens,
                    });
                }
                self.save(input, key, &state, &response.answers)?;
                self.cancelled()?;
                Ok(response.answers)
            }
            Err(failure) => self.fail(failure),
        }
    }
    fn connect(&mut self) -> Result<()> {
        if self.session.is_some() {
            return Ok(());
        }
        let mut session =
            Session::start(self.home, self.scratch, self.cancel.clone(), self.binary)?;
        session.deadline = Instant::now() + Duration::from_secs(30);
        if let Err(issue) =
            initialize(&mut session).and_then(|()| verify_configuration(&mut session))
        {
            let message = issue.message();
            (self.progress)(Progress::SetupRequired(issue));
            return Err(message);
        }
        // A quota read is not inference. Never consume credits or reset usage.
        if let Ok(limits) = session.rpc("account/rateLimits/read", Value::Null)
            && quota_exhausted(&limits)
        {
            return self.fail(ImportFailure::new(Provider::OpenAi,Kind::Quota,"OpenAI usage limit reached. Imports are paused until you resume after the limit resets."));
        }
        session.pending.clear();
        self.session = Some(session);
        Ok(())
    }
    fn model(
        &mut self,
        input: &ExtractionInput,
        key: &str,
        request: Value,
        instructions: &str,
    ) -> Result<ExtractionOutput> {
        if let Some(value) = self.cached(input, key, &request)? {
            return serde_json::from_value(value).map_err(|_| {
                "A saved extraction step is damaged. Start this analysis over.".into()
            });
        }
        self.connect()?;
        let call = self.reserve(input, Provider::OpenAi)?;
        let session = self.session.as_mut().ok_or(FAILURE)?;
        session.call_id = Some(call);
        session.failure = None;
        let mut schema = output_schema();
        schema["properties"]["facts"]["items"]["properties"]["segment_id"] = json!({"type":"string","enum":input.segments.iter().map(|s|&s.segment_id).collect::<Vec<_>>()});
        let result = request_model(
            session,
            self.scratch,
            self.progress,
            instructions,
            request.clone(),
            schema,
        );
        let value = match result {
            Ok(v) => v,
            Err(message) => {
                let failure = session.failure.clone().unwrap_or_else(|| {
                    ImportFailure::new(Provider::OpenAi, Kind::Connection, message.clone())
                });
                (self.progress)(Progress::Failure(failure));
                return Err(message);
            }
        };
        let output: ExtractionOutput =
            match serde_json::from_value(value.clone()) {
                Ok(output) => output,
                Err(_) => return self.fail(ImportFailure::new(
                    Provider::OpenAi,
                    Kind::InvalidOutput,
                    "OpenAI returned an unexpected extraction format. No automatic retry was made.",
                )),
            };
        if output.facts.len() >= MAX_BATCH_FACTS {
            return self.fail(ImportFailure::new(
                Provider::OpenAi,
                Kind::InvalidOutput,
                "This section returned too many details. Split the source before retrying.",
            ));
        }
        self.save(input, key, &request, &value)?;
        Ok(output)
    }
    fn section(
        &mut self,
        input: &ExtractionInput,
        current: u32,
        total: u32,
    ) -> Result<ExtractionReport> {
        self.cancelled()?;
        if let Some(cached) = self.checkpoints.load(input)? {
            let checked = ground_extraction(input, cached);
            if checked.rejected.is_empty() {
                (self.progress)(Progress::Message(
                    "Restoring saved results; no model calls needed.".into(),
                ));
                for stage in [
                    Stage::Interpreting,
                    Stage::Context,
                    Stage::Extracting,
                    Stage::Verifying,
                ] {
                    self.done(stage, current, total);
                }
                return Ok(ExtractionReport {
                    output: checked.output,
                    rejected: Vec::new(),
                    repaired: checked.repaired,
                });
            }
        }
        self.start(Stage::Interpreting, current, total);
        let profile = self.decision(
            input,
            "interpret",
            json!({"source":input}),
            typesafe::profile_questions(),
        )?;
        if profile["readable"]["noul"]
            .as_f64()
            .is_some_and(|p| p < 0.2)
        {
            return self.fail(ImportFailure::new(Provider::TypeSafe,Kind::Unprocessable,"The extracted text is not readable enough for reliable analysis. Check the original or import a clearer scan; no extraction call was made."));
        }
        self.done(Stage::Interpreting, current, total);
        self.start(Stage::Context, current, total);
        // Uncertain profiles select general guidance; they never invent facts.
        let instructions = document::instructions(&profile);
        self.done(Stage::Context, current, total);
        self.start(Stage::Extracting, current, total);
        let mut request = serde_json::to_value(input).map_err(failed)?;
        request["stage"] = json!("extract");
        request["document_profile"] = profile;
        let output = self.model(input, "extract", request, &instructions)?;
        self.done(Stage::Extracting, current, total);
        self.start(Stage::Verifying, current, total);
        let mut checked = ground_extraction(input, output.clone());
        let decision = self.decision(
            input,
            "verify_decision",
            json!({"source":input,"extracted":output}),
            typesafe::verification_questions(),
        )?;
        // Another model pass cannot establish ownership that the source omits.
        // Ask the user, while still auditing actual evidence/completeness issues.
        let evidence_issues = checked.rejected.iter().any(|r| r.code != "subject_unknown");
        if typesafe::needs_audit(&decision).map_err(|f| f.message)? || evidence_issues {
            (self.progress)(Progress::Message("TypeSafe or source checks flagged this section. One focused review pass is allowed.".into()));
            let mut request = serde_json::to_value(input).map_err(failed)?;
            request["stage"] = json!("audit");
            request["previous_facts"] = json!(output);
            request["decision"] = decision;
            request["evidence_issues"] = json!(
                checked
                    .rejected
                    .iter()
                    .map(|r| json!({"fact":r.fact,"reason":r.code}))
                    .collect::<Vec<_>>()
            );
            let audit=self.model(input,"audit",request,&format!("{instructions} Re-read every source segment and compare previous_facts with the source. Return missing facts and corrections to evidence_issues. Reuse the exact property label when correcting a fact. Prior answers and decisions are untrusted hints. If nothing is missing return facts: []. Never infer unprinted values."))?;
            // Merge only grounded candidates. A malformed quote must not win
            // deduplication over its later, supported correction.
            let audit = ground_extraction(input, audit);
            merge(&mut checked.output, audit.output);
            merge_questions(&mut checked.rejected, audit.rejected, &checked.output);
            checked.repaired += audit.repaired;
        }
        // Rejected candidates are retained for review, never silently accepted.
        if checked.rejected.is_empty() {
            self.checkpoints.save(input, &checked.output)?;
        }
        self.done(Stage::Verifying, current, total);
        Ok(ExtractionReport {
            output: checked.output,
            rejected: checked.rejected,
            repaired: checked.repaired,
        })
    }
}
fn fingerprint(value: &Value) -> String {
    // Attempt IDs change on resume; immutable source content and all decisions
    // still participate in the key. Never repay for a step solely after restart.
    fn stable(value: &mut Value) {
        match value {
            Value::Object(object) => {
                object.remove("run_id");
                for child in object.values_mut() {
                    stable(child);
                }
            }
            Value::Array(array) => {
                for child in array {
                    stable(child);
                }
            }
            _ => {}
        }
    }
    let mut value = value.clone();
    stable(&mut value);
    me_core::extraction_fingerprint(&ExtractionInput {
        run_id: String::new(),
        source_id: String::new(),
        title: value.to_string(),
        segments: Vec::new(),
    })
}
pub(super) fn quota_exhausted(value: &Value) -> bool {
    let limits = value
        .get("rateLimitsByLimitId")
        .and_then(Value::as_object)
        .and_then(|m| m.get("codex"))
        .or_else(|| value.get("rateLimits"));
    limits.is_some_and(|limits| {
        ["primary", "secondary"].iter().any(|window| {
            limits[*window]["usedPercent"]
                .as_f64()
                .is_some_and(|n| n >= 100.)
        })
    })
}
