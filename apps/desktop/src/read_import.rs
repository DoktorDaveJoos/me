//! Worker side of deep-first reading, for automatic and manual runs alike:
//! Classify (TypeSafe) → guided reader (OpenAI, grounded) → omission sweep and
//! one focused audit → local typing → TypeSafe verification → Resolve and the
//! read store. Runs on a background thread; the vault mutex is held only for
//! short reads and writes, never across provider requests.
use super::*;
use me_agent::codex::Progress;
use me_agent::fact_verification as verify;
use me_agent::graph_pipeline::{self as pipeline, Classification};
use me_agent::read_assembly::{
    covered_spans, keep_grounded, named_owners, profile_values, read_fact, time_year, type_facts,
    unverified_fact,
};
use me_core::{
    DocumentGraph, DocumentRead, ImportErrorKind as Kind, ImportFailure, ImportProvider,
    ImportStage, SourceSegment, TypingContext, find_candidates, uncovered,
};
use me_diagnostics::{Field as F, record};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::Sender,
};

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
        let mut open = uncovered(&candidates, &covered_spans(&segments, &facts));
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
            open = uncovered(&candidates, &covered_spans(&segments, &facts));
        }
        if cancel.load(Ordering::SeqCst) {
            return Err("Analysis stopped. Saved steps are kept.".into());
        }

        // 5. Local typing.
        let typing = TypingContext::new(&segments, classification.doc_type.as_deref(), year);
        let (typed, inputs) = type_facts(&facts, &segments, &typing, kind);

        // 6. TypeSafe verification. A failure keeps the facts unverified and resumable.
        step(tx, session, item, run, ImportStage::Verifying, false);
        let named = named_owners(&candidates);
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
