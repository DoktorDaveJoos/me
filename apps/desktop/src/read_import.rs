//! Worker side of deep-first reading, for automatic and manual runs alike:
//! Classify (TypeSafe) → guided reader (OpenAI, grounded) → omission sweep and
//! one focused audit → local typing → TypeSafe verification → Resolve and the
//! read store. Runs on a background thread; the vault mutex is held only for
//! short reads and writes, never across provider requests.
use super::*;
use me_agent::codex::Progress;
use me_agent::fact_verification::{self as verify, Band, FactInput, Owner, PeriodKind, Verdict};
use me_agent::graph_pipeline::{self as pipeline, Classification};
use me_core::{
    Candidate, CandidateKind, CandidateValue, ConfidenceSource, DocType, DocumentGraph,
    DocumentRead, ExtractedFact, FactState, ImportErrorKind as Kind, ImportFailure, ImportProvider,
    ImportStage, Located, ReadFact, RejectedFact, SlotContent, SlotValue, SourceSegment,
    TypingContext, ValueKind, doc_types::MrzField, find_candidates, locate, type_value, uncovered,
};
use me_diagnostics::{Field as F, record};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::Sender,
};

/// Other people named in a document offered as owners, at most.
const NAMED_OWNERS: usize = 24;

pub(super) struct ReadRun {
    pub outcome: me_core::ReadOutcome,
}

fn vault_call<T>(
    session: &Arc<Mutex<Option<Vault>>>,
    f: impl FnOnce(&mut Vault) -> me_core::Result<T>,
) -> Result<T, String> {
    let mut guard = session
        .lock()
        .map_err(|_| "Vault unavailable.".to_string())?;
    let vault = guard.as_mut().ok_or("Vault locked.".to_string())?;
    f(vault).map_err(|e| e.to_string())
}

/// Reserves each TypeSafe request durably before it is sent, then records usage.
struct Metered<'a> {
    session: &'a Arc<Mutex<Option<Vault>>>,
    item: u64,
    run: &'a str,
    inner: me_agent::typesafe::TypeSafe,
    tx: &'a Sender<Progress>,
}
impl me_agent::typesafe::Decisions for Metered<'_> {
    fn evaluate(
        &mut self,
        state: serde_json::Value,
        questions: serde_json::Value,
        cancel: &AtomicBool,
    ) -> me_agent::typesafe::Result<me_agent::typesafe::DecisionResponse> {
        let call = vault_call(self.session, |v| {
            v.reserve_graph_request(self.item, self.run, ImportProvider::TypeSafe)
        })
        .map_err(|e| ImportFailure::new(ImportProvider::Local, Kind::Unprocessable, e))?;
        let response = self.inner.evaluate(state, questions, cancel)?;
        let _ = vault_call(self.session, |v| {
            v.record_import_usage(
                self.item,
                self.run,
                &call,
                response.input_tokens.unwrap_or(0),
                response.output_tokens.unwrap_or(0),
            )
        });
        let _ = self.tx.send(Progress::Usage {
            call,
            input_tokens: response.input_tokens.unwrap_or(0),
            output_tokens: response.output_tokens.unwrap_or(0),
        });
        Ok(response)
    }
}

fn step(
    tx: &Sender<Progress>,
    session: &Arc<Mutex<Option<Vault>>>,
    item: u64,
    run: &str,
    stage: ImportStage,
    done: bool,
) {
    let (current, total) = (u32::from(done), 1);
    let _ = vault_call(session, |v| {
        v.update_import_progress(item, run, stage, current, total)
    });
    let _ = tx.send(Progress::Step {
        stage,
        current,
        total,
    });
    let _ = tx.send(Progress::Stage(stage));
}

/// One import attempt: where its progress, usage and failures are recorded.
struct Attempt<'a> {
    session: &'a Arc<Mutex<Option<Vault>>>,
    item: u64,
    run: &'a str,
    cancel: &'a Arc<AtomicBool>,
    tx: &'a Sender<Progress>,
    /// A provider failure was recorded for this attempt; a later generic local
    /// failure must not replace it.
    failure_recorded: Cell<bool>,
}
impl Attempt<'_> {
    /// Records a typed provider failure and returns its message.
    fn fail(&self, failure: ImportFailure) -> String {
        self.failure_recorded.set(true);
        let _ = vault_call(self.session, |v| {
            v.record_import_failure(self.item, self.run, &failure)
        });
        let message = failure.message.clone();
        let _ = self.tx.send(Progress::Failure(failure));
        message
    }

    /// Forwards reader progress into the vault and the UI channel; a failed write
    /// stops the attempt instead of continuing unrecorded.
    fn forward(&self, failed: &mut Option<String>, event: Progress) {
        let (session, item, run) = (self.session, self.item, self.run);
        let saved = (|| -> Result<(), String> {
            let active = match &event {
                Progress::Step {
                    stage,
                    current,
                    total,
                } => vault_call(session, |v| {
                    v.update_import_progress(item, run, *stage, *current, *total)
                })?,
                Progress::Failure(failure) => {
                    self.failure_recorded.set(true);
                    vault_call(session, |v| v.record_import_failure(item, run, failure))?;
                    true
                }
                // Let an already-paid response finish and reach its checkpoint. The
                // durable reservation gate blocks the next request.
                Progress::Usage {
                    call,
                    input_tokens,
                    output_tokens,
                } => vault_call(session, |v| {
                    v.record_import_usage(item, run, call, *input_tokens, *output_tokens)
                })?,
                _ => true,
            };
            if active {
                Ok(())
            } else {
                Err("This import attempt is no longer active.".into())
            }
        })();
        if let Err(e) = saved {
            *failed = Some(e);
            self.cancel.store(true, Ordering::SeqCst);
        }
        let _ = self.tx.send(event);
    }
}

/// Reads one document end to end. A failure that no provider reported is
/// recorded as a local storage failure, or as cancelled after a stop.
pub(super) fn run_read(
    session: &Arc<Mutex<Option<Vault>>>,
    item: u64,
    run: &str,
    cancel: &Arc<AtomicBool>,
    tx: &Sender<Progress>,
    home: &std::path::Path,
) -> Result<ReadRun, String> {
    let attempt = Attempt {
        session,
        item,
        run,
        cancel,
        tx,
        failure_recorded: Cell::new(false),
    };
    let result = read(&attempt, home);
    if let Err(e) = &result
        && !attempt.failure_recorded.get()
    {
        let kind = if cancel.load(Ordering::SeqCst) {
            Kind::Cancelled
        } else {
            Kind::Storage
        };
        let failure = ImportFailure::new(ImportProvider::Local, kind, e.clone());
        let _ = vault_call(session, |v| v.record_import_failure(item, run, &failure));
    }
    result
}

fn read(a: &Attempt<'_>, home: &std::path::Path) -> Result<ReadRun, String> {
    let (session, item, run, cancel, tx) = (a.session, a.item, a.run, a.cancel, a.tx);
    // 1. Inputs.
    let (source, title, texts, anchors, threshold, fingerprint, batch) =
        vault_call(session, |v| {
            let (source, title, texts) = v.document_texts(item)?;
            let fingerprint = v.source_fingerprint(&source)?;
            Ok((
                source,
                title,
                texts,
                v.identity_anchors()?,
                v.check_threshold()?,
                fingerprint,
                v.item_batch(item)?,
            ))
        })?;
    let segments: Vec<SourceSegment<'_>> = texts
        .iter()
        .map(|t| SourceSegment {
            id: &t.segment_id,
            text: &t.text,
        })
        .collect();
    let names = anchors.names();
    let name_refs: Vec<&str> = names.iter().map(|(_, n)| n.as_str()).collect();
    let year = time_year();
    let candidates = find_candidates(&segments, &name_refs, year);
    let mut decisions = Metered {
        session,
        item,
        run,
        inner: me_agent::typesafe::TypeSafe::configured().map_err(|f| a.fail(f))?,
        tx,
    };
    let mut requests = 0u32;
    let mut tokens = 0u64;

    // 2. Classify (cached by content), so every read document is filed.
    step(tx, session, item, run, ImportStage::Interpreting, false);
    let classify_version = pipeline::classify_version(&anchors);
    let cached = vault_call(session, |v| {
        v.stage_output(&fingerprint, pipeline::CLASSIFY_STAGE, &classify_version)
    })?
    .and_then(|v| serde_json::from_value::<Classification>(v).ok());
    let classification = match cached {
        Some(c) => c,
        None => {
            let document = pipeline::Document {
                title: &title,
                segments: &segments,
            };
            let c = pipeline::classify(document, &anchors, &candidates, &mut decisions, cancel)
                .map_err(|f| a.fail(f))?;
            requests += c.usage.requests;
            tokens += c.usage.input_tokens;
            let model = c.usage.models.first().cloned();
            let value = serde_json::to_value(&c).map_err(|_| "Invalid classification.")?;
            vault_call(session, |v| {
                v.save_stage_output(
                    &fingerprint,
                    pipeline::CLASSIFY_STAGE,
                    &classify_version,
                    &value,
                    model.as_deref(),
                    Some(c.usage.input_tokens),
                )
            })?;
            c
        }
    };
    vault_call(session, |v| {
        v.save_document_profile(
            &source,
            &classification.family,
            classification.family_confidence,
            classification.doc_type.as_deref(),
            classification.type_confidence,
            classification.subject.as_deref(),
            classification.subject_name.as_deref(),
            classification.usage.models.first().map(String::as_str),
        )
    })?;
    step(tx, session, item, run, ImportStage::Interpreting, true);
    // Only an eager registry type fills the profile; every type is read in full.
    let kind = classification.eager_type();
    let doc_label = classification
        .doc_type
        .as_deref()
        .and_then(me_core::doc_type)
        .map_or("Document", |t| t.label);
    let guide =
        me_agent::guides::reading_guide(&classification.family, classification.doc_type.as_deref());

    // 3–8 run inside one extraction run: any failure leaves it failed and resumable.
    let input = vault_call(session, |v| {
        v.prepare_extraction(item, me_agent::codex::INBOX_MODEL)
    })?;
    let result = (|| -> Result<ReadRun, String> {
        // 3. Guided reading over the whole document, checkpointed per guide.
        let mut checkpoints = super::ai_ui::VaultCheckpoints {
            session: session.clone(),
            pipeline: format!("{}|{}", me_agent::codex::PIPELINE, guide.key),
        };
        let mut failed = None;
        let report = me_agent::codex::extract_document(
            home,
            &input,
            &guide,
            cancel.clone(),
            |e| a.forward(&mut failed, e),
            &mut checkpoints,
        );
        if let Some(e) = failed.take() {
            return Err(e);
        }
        let report = report?;
        let mut facts = report.output.facts;
        let mut rejected = BTreeMap::<String, usize>::new();
        keep_grounded(&mut facts, &mut rejected, report.rejected);

        // 4. Omission sweep and at most one focused audit.
        let covered = |facts: &[ExtractedFact]| -> Vec<(String, usize, usize)> {
            facts
                .iter()
                .filter_map(|f| locate(&segments, &f.segment_id, &f.quote, &f.value))
                .map(|l| (l.segment_id, l.quote_start, l.quote_end))
                .collect()
        };
        let mut open = uncovered(&candidates, &covered(&facts));
        if !open.is_empty() {
            let _ = tx.send(Progress::Message(format!(
                "Checking {} value(s) the first pass did not quote",
                open.len()
            )));
            let previous = me_core::ExtractionOutput {
                facts: facts.clone(),
            };
            let audit = me_agent::codex::audit_document(
                home,
                &input,
                &guide,
                &previous,
                &open,
                cancel.clone(),
                |e| a.forward(&mut failed, e),
                &mut checkpoints,
            );
            if let Some(e) = failed.take() {
                return Err(e);
            }
            let audit = audit?;
            for f in audit.output.facts {
                me_core::merge_extracted_fact(&mut facts, f);
            }
            keep_grounded(&mut facts, &mut rejected, audit.rejected);
            open = uncovered(&candidates, &covered(&facts));
        }
        if cancel.load(Ordering::SeqCst) {
            return Err("Analysis stopped. Saved steps are kept.".into());
        }

        // 5. Local typing.
        let typing = TypingContext::new(&segments, classification.doc_type.as_deref(), year);
        let (typed, inputs) = type_facts(&facts, &segments, &typing, kind);

        // 6. TypeSafe verification. A failure keeps the facts unverified and resumable.
        step(tx, session, item, run, ImportStage::Verifying, false);
        let mut named: Vec<String> = Vec::new();
        for c in candidates
            .iter()
            .filter(|c| c.kind == CandidateKind::PersonName)
        {
            let name = c.text.trim();
            if named.len() < NAMED_OWNERS && !named.iter().any(|n| n == name) {
                named.push(name.to_owned());
            }
        }
        let subject = classification
            .subject
            .clone()
            .or_else(|| Some(anchors.self_entity.clone()));
        let excerpt = segments.first().map_or("", |s| s.text);
        let verification = match verify::verify_facts(
            &title,
            doc_label,
            excerpt,
            &inputs,
            &anchors,
            &named,
            subject.as_deref(),
            threshold,
            &mut decisions,
            cancel,
        ) {
            Ok(v) => v,
            Err(failure) => {
                let read = DocumentRead {
                    run_id: input.run_id.clone(),
                    graph: None,
                    facts: typed
                        .iter()
                        .zip(&inputs)
                        .map(|(t, i)| unverified_fact(t, i))
                        .collect(),
                    uninterpreted: open,
                    rejected,
                };
                let _ = vault_call(session, |v| v.apply_read(&source, &read));
                return Err(a.fail(failure));
            }
        };
        requests += verification.usage.requests;
        tokens += verification.usage.input_tokens;

        // 7. Profile values: MRZ fields by position, verified mappings, pay month.
        let graph = kind.map(|k| {
            let mut models = classification.usage.models.clone();
            for m in &verification.usage.models {
                if !models.contains(m) {
                    models.push(m.clone());
                }
            }
            DocumentGraph {
                doc_type: k.key.into(),
                subject: subject.clone(),
                subject_confidence: classification.subject_confidence.max(0.5),
                values: profile_values(
                    k,
                    &candidates,
                    &segments,
                    &typed,
                    &inputs,
                    &verification.verdicts,
                ),
                models,
                correction: verification.correction,
            }
        });

        // 8. Persist.
        let read = DocumentRead {
            run_id: input.run_id.clone(),
            graph,
            facts: typed
                .iter()
                .zip(&inputs)
                .zip(&verification.verdicts)
                .map(|((t, i), v)| read_fact(t, i, v, &anchors.self_entity))
                .collect(),
            uninterpreted: open,
            rejected,
        };
        let outcome = vault_call(session, |v| v.apply_read(&source, &read))?;
        vault_call(session, |v| v.complete_read_run(&input.run_id))?;
        step(tx, session, item, run, ImportStage::Verifying, true);
        record(
            "read.stored",
            &[
                F::Count("item", item),
                F::Count("facts", facts.len() as u64),
                F::Count("located", typed.len() as u64),
                F::Count("values", outcome.values as u64),
                F::Count("in_profile", outcome.in_profile as u64),
                F::Count("checks", outcome.checks as u64),
                F::Count("uninterpreted", outcome.uninterpreted as u64),
                F::Flag("eager", kind.is_some()),
            ],
        );
        Ok(ReadRun { outcome })
    })();
    if result.is_err() {
        let _ = vault_call(session, |v| v.fail_extraction(&input.run_id));
    }
    if let Some(batch) = &batch
        && (requests > 0 || tokens > 0)
    {
        let _ = vault_call(session, |v| {
            v.record_batch_usage(batch, requests, tokens, 0)
        });
    }
    result
}

/// Facts whose only open question is their owner stay (verification asks it);
/// every other grounding rejection is counted by code, never stored.
fn keep_grounded(
    facts: &mut Vec<ExtractedFact>,
    rejected: &mut BTreeMap<String, usize>,
    incoming: Vec<RejectedFact>,
) {
    for r in incoming {
        if r.code == "subject_unknown" {
            me_core::merge_extracted_fact(facts, r.fact);
        } else {
            *rejected.entry(r.code.to_owned()).or_default() += 1;
        }
    }
}

/// A reader fact at its exact source span, with its value typed for the slot it
/// was tagged with when the printed value can hold that slot's kind.
struct Typed<'f> {
    fact: &'f ExtractedFact,
    at: Located,
    candidate: Option<Candidate>,
}

/// The printed label of a reader property.
fn fact_label(property: &str) -> String {
    me_core::document_field_label(property)
        .map(str::to_owned)
        .or_else(|| {
            me_core::STANDARD_FIELDS
                .iter()
                .find(|s| s.key == property)
                .map(|s| s.label.to_owned())
        })
        .unwrap_or_else(|| property.to_owned())
}

/// Locates and types every reader fact. Facts whose quote or value is not
/// verbatim in their segment are left out. A slot tag is kept only when the value
/// type-checks for it, or when it names a category code could not map.
fn type_facts<'f>(
    facts: &'f [ExtractedFact],
    segments: &[SourceSegment<'_>],
    typing: &TypingContext,
    kind: Option<&'static DocType>,
) -> (Vec<Typed<'f>>, Vec<FactInput>) {
    let mut typed = Vec::new();
    let mut inputs = Vec::new();
    for fact in facts {
        let Some(at) = locate(segments, &fact.segment_id, &fact.quote, &fact.value) else {
            continue;
        };
        let label = fact_label(&fact.property);
        // Machine-readable-zone slots are filled by position, never from a tag.
        let slot = kind
            .and_then(|k| k.slot(&fact.slot))
            .filter(|s| s.mrz.is_none());
        let candidate = slot.and_then(|s| type_value(typing, &at, &label, &fact.value, s.value));
        let category_unmapped =
            matches!(slot.map(|s| s.value), Some(ValueKind::Category(_))) && candidate.is_none();
        let slot = slot.filter(|_| candidate.is_some() || category_unmapped);
        inputs.push(FactInput {
            label,
            value: fact.value.clone(),
            quote: fact.quote.clone(),
            line: at.line.clone(),
            money: me_core::amount_of(typing, &fact.value).is_some(),
            slot,
            category_unmapped,
        });
        typed.push(Typed {
            fact,
            at,
            candidate,
        });
    }
    (typed, inputs)
}

/// Accepted assumptions are verified; a check or a rejected assumption keeps the
/// grounded fact on its document as uncertain.
fn fact_state(band: Band) -> FactState {
    match band {
        Band::Accept => FactState::Verified,
        Band::Check | Band::Reject => FactState::Uncertain,
    }
}

fn period_key(period: PeriodKind) -> &'static str {
    match period {
        PeriodKind::Document => "document",
        PeriodKind::Cumulative => "cumulative",
        PeriodKind::Other => "other",
        PeriodKind::NotPeriodic => "none",
    }
}

/// The stored document fact of one verified reader fact. It is linked to the
/// profile only when its mapping was verified.
fn read_fact(t: &Typed<'_>, input: &FactInput, v: &Verdict, self_entity: &str) -> ReadFact {
    let (owner, owner_entity, owner_name) = match &v.owner {
        Owner::Anchor(e) if e == self_entity => ("self", Some(e.clone()), None),
        Owner::Anchor(e) => ("household", Some(e.clone()), None),
        Owner::Party(name) => ("party", None, Some(name.clone())),
        Owner::Organization => ("organization", None, None),
        Owner::Unclear => ("unclear", None, None),
    };
    ReadFact {
        label: input.label.clone(),
        value: input.value.clone(),
        segment_id: t.at.segment_id.clone(),
        start: t.at.start,
        end: t.at.end,
        context: t.fact.context_quote.clone(),
        owner: owner.into(),
        owner_entity,
        owner_name,
        period: v.period.map(|p| period_key(p).to_owned()),
        slot: input.slot.filter(|_| v.mapped).map(|s| s.key.to_owned()),
        state: fact_state(v.band),
        confidence: Some(v.confidence),
    }
}

/// A grounded fact whose assumptions could not be verified: kept on its
/// document, owner unknown, never in the profile.
fn unverified_fact(t: &Typed<'_>, input: &FactInput) -> ReadFact {
    ReadFact {
        label: input.label.clone(),
        value: input.value.clone(),
        segment_id: t.at.segment_id.clone(),
        start: t.at.start,
        end: t.at.end,
        context: t.fact.context_quote.clone(),
        owner: "unknown".into(),
        owner_entity: None,
        owner_name: None,
        period: None,
        slot: None,
        state: FactState::Unverified,
        confidence: None,
    }
}

/// Code applies the direction TypeSafe judged: a refund is stored negative and
/// a payment positive, whatever sign was printed. Zero stays unsigned.
fn apply_direction(c: &mut Candidate, refund: Option<bool>) {
    let (Some(refund), CandidateValue::Money { amount, .. }) = (refund, &mut c.value) else {
        return;
    };
    let magnitude = amount.trim_start_matches(['-', '+']).to_owned();
    let zero = magnitude.bytes().all(|b| b == b'0' || b == b'.');
    *amount = if refund && !zero {
        format!("-{magnitude}")
    } else {
        magnitude
    };
}

/// Profile values of an eager type: machine-readable-zone fields by position,
/// then every verified mapping, then the pay-month fallback.
fn profile_values(
    kind: &DocType,
    candidates: &[Candidate],
    segments: &[SourceSegment<'_>],
    typed: &[Typed<'_>],
    inputs: &[FactInput],
    verdicts: &[Verdict],
) -> Vec<SlotValue> {
    let mut values: Vec<SlotValue> = Vec::new();
    if let Some(mrz) = candidates
        .iter()
        .find(|c| matches!(&c.value, CandidateValue::Mrz(m) if m.check_digits_valid))
    {
        for slot in kind.slots {
            if let Some(field) = slot.mrz
                && let Some(c) = mrz_value(mrz, field)
            {
                values.push(SlotValue {
                    slot: slot.key.into(),
                    content: SlotContent::Candidate(Box::new(c)),
                    period: None,
                    confidence: 1.,
                    source: ConfidenceSource::Checksum,
                    value_checked: true,
                    check: false,
                });
            }
        }
    }
    for ((t, input), v) in typed.iter().zip(inputs).zip(verdicts) {
        let (Some(slot), true) = (input.slot, v.mapped) else {
            continue;
        };
        if values.iter().any(|x| x.slot == slot.key) {
            continue;
        }
        let content = match (&t.candidate, &v.category) {
            (None, Some(key)) if input.category_unmapped => {
                SlotContent::Category { key: key.clone() }
            }
            (Some(c), _) => {
                let mut c = c.clone();
                if slot.value == ValueKind::Balance {
                    apply_direction(&mut c, v.refund);
                }
                SlotContent::Candidate(Box::new(c))
            }
            _ => continue,
        };
        values.push(SlotValue {
            slot: slot.key.into(),
            content,
            period: v.payment_period,
            confidence: v.confidence,
            source: ConfidenceSource::Typesafe,
            value_checked: t.candidate.as_ref().is_some_and(|c| c.checksum),
            check: v.band == Band::Check,
        });
    }
    if let Some(slot) = kind.slots.iter().find(|s| s.value == ValueKind::Period)
        && !values.iter().any(|x| x.slot == slot.key)
        && let Some((period, confidence)) =
            shared_period(candidates, segments, typed, inputs, verdicts)
    {
        values.push(SlotValue {
            slot: slot.key.into(),
            content: SlotContent::Candidate(Box::new(period)),
            period: None,
            confidence,
            source: ConfidenceSource::Typesafe,
            value_checked: false,
            check: false,
        });
    }
    values
}

/// Where a fact's context quote sits: in the fact's own segment, else in the
/// first segment that prints it.
fn locate_context(segments: &[SourceSegment<'_>], fact: &ExtractedFact) -> Option<Located> {
    let quote = fact.context_quote.as_str();
    if quote.trim().is_empty() {
        return None;
    }
    locate(segments, &fact.segment_id, quote, quote).or_else(|| {
        segments
            .iter()
            .find_map(|s| locate(segments, s.id, quote, quote))
    })
}

/// Pay-month fallback: the one period that the context quotes of all mapped
/// amounts for this document's own period print, at its real source span, with
/// the weakest confidence among those amounts. `None` when they print no period
/// or more than one.
fn shared_period(
    candidates: &[Candidate],
    segments: &[SourceSegment<'_>],
    typed: &[Typed<'_>],
    inputs: &[FactInput],
    verdicts: &[Verdict],
) -> Option<(Candidate, f64)> {
    let mut found: Vec<&Candidate> = Vec::new();
    let mut confidence: f64 = 1.;
    for ((t, input), v) in typed.iter().zip(inputs).zip(verdicts) {
        if !(v.mapped && input.money && v.period == Some(PeriodKind::Document)) {
            continue;
        }
        let Some(at) = locate_context(segments, t.fact) else {
            continue;
        };
        let periods: Vec<&Candidate> = candidates
            .iter()
            .filter(|c| {
                c.kind == CandidateKind::Period
                    && c.segment_id == at.segment_id
                    && at.quote_start <= c.start
                    && c.end <= at.quote_end
            })
            .collect();
        if !periods.is_empty() {
            confidence = confidence.min(v.confidence);
            found.extend(periods);
        }
    }
    let first = *found.first()?;
    found
        .iter()
        .all(|c| c.value == first.value)
        .then(|| (first.clone(), confidence))
}

/// The value of one machine-readable-zone field, at the zone's span. Position
/// defines the meaning; the check digits validate the value.
fn mrz_value(c: &Candidate, field: MrzField) -> Option<Candidate> {
    let CandidateValue::Mrz(m) = &c.value else {
        return None;
    };
    if !m.check_digits_valid {
        return None;
    }
    let value = match field {
        MrzField::DocumentNumber => CandidateValue::Identifier(m.document_number.clone()),
        MrzField::BirthDate => CandidateValue::Date(m.birth_date.clone()?),
        MrzField::ExpiryDate => CandidateValue::Date(m.expiry_date.clone()?),
        MrzField::Nationality => CandidateValue::Identifier(m.nationality.clone()),
    };
    Some(Candidate { value, ..c.clone() })
}

/// Current UTC year, for the century of two-digit MRZ years.
fn time_year() -> i32 {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400) as i64;
    // Civil-from-days (Howard Hinnant), year only.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    (yoe + era * 400 + i64::from(mp >= 10)) as i32
}

#[cfg(test)]
#[path = "read_import_tests.rs"]
mod tests;
