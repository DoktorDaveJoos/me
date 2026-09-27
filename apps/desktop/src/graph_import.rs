//! Worker side of the tiered import pipeline for automatically queued documents:
//! Classify (TypeSafe) → local candidates → Extract (TypeSafe selection) → optional
//! OpenAI gap fill verified by TypeSafe → Resolve. Runs on a background thread; the
//! vault mutex is held only for short reads and writes, never across network calls.
use super::*;
use me_agent::codex::Progress;
use me_agent::graph_pipeline::{self as pipeline, Classification, Extraction};
use me_core::{
    ImportErrorKind as Kind, ImportFailure, ImportProvider, ImportStage, SourceSegment,
    find_candidates,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::Sender,
};

/// Bytes of source text offered to one gap-fill call (one small section).
const GAP_FILL_BYTES: usize = 3_200;

pub(super) struct GraphRun {
    pub assertions: usize,
    pub checks: usize,
    pub eager: bool,
    pub family: String,
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

/// Gap fill reservations count against the file allowance and the batch allowance.
struct GapCheckpoints<'a> {
    session: &'a Arc<Mutex<Option<Vault>>>,
    item: u64,
    run: &'a str,
}
impl me_agent::codex::Checkpoints for GapCheckpoints<'_> {
    fn load(
        &mut self,
        _: &me_core::ExtractionInput,
    ) -> Result<Option<me_core::ExtractionOutput>, String> {
        Ok(None)
    }
    fn save(
        &mut self,
        _: &me_core::ExtractionInput,
        _: &me_core::ExtractionOutput,
    ) -> Result<(), String> {
        Ok(())
    }
    fn reserve(
        &mut self,
        _: &me_core::ExtractionInput,
        provider: ImportProvider,
    ) -> Result<String, ImportFailure> {
        vault_call(self.session, |v| {
            v.reserve_graph_request(self.item, self.run, provider)
        })
        .map_err(|e| ImportFailure::new(provider, Kind::Unprocessable, e))
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

/// Converts grounded facts of another model into candidates: typed values parsed
/// from each exact quote, plus the printed value itself.
fn gap_candidates(
    report: &me_agent::codex::ExtractionReport,
    segments: &[me_core::SourceText],
    names: &[&str],
    year: i32,
) -> Vec<me_core::Candidate> {
    let mut out = Vec::new();
    for fact in &report.output.facts {
        let Some(segment) = segments.iter().find(|s| s.segment_id == fact.segment_id) else {
            continue;
        };
        let Some(offset) = segment.text.find(&fact.quote) else {
            continue;
        };
        let line = segment.text[..offset].rfind('\n').map_or(0, |i| i + 1);
        let line_end = segment.text[offset..]
            .find('\n')
            .map_or(segment.text.len(), |i| offset + i);
        let line = segment.text[line..line_end].trim().to_owned();
        let quote = [SourceSegment {
            id: &fact.segment_id,
            text: &fact.quote,
        }];
        for mut c in find_candidates(&quote, names, year) {
            c.start += offset;
            c.end += offset;
            c.line.clone_from(&line);
            out.push(c);
        }
        if let Some(start) = fact.quote.find(&fact.value) {
            out.push(me_core::Candidate {
                id: String::new(),
                kind: me_core::CandidateKind::LabelValue,
                text: fact.value.clone(),
                value: me_core::CandidateValue::Text(fact.value.clone()),
                segment_id: fact.segment_id.clone(),
                start: offset + start,
                end: offset + start + fact.value.len(),
                line: line.clone(),
                label: Some(fact.property.clone()),
                checksum: false,
            });
        }
    }
    for (i, c) in out.iter_mut().enumerate() {
        c.id = format!("g{i}");
    }
    out
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_graph_import(
    session: &Arc<Mutex<Option<Vault>>>,
    item: u64,
    run: &str,
    cancel: &Arc<AtomicBool>,
    tx: &Sender<Progress>,
    codex_home: &std::path::Path,
    codex_ready: bool,
) -> Result<GraphRun, String> {
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
    let document = pipeline::Document {
        title: &title,
        segments: &segments,
    };
    let names = anchors.names();
    let name_refs: Vec<&str> = names.iter().map(|(_, n)| n.as_str()).collect();
    let year = time_year();
    // Local, free: candidate values with their exact source spans.
    step(tx, session, item, run, ImportStage::Context, false);
    let candidates = find_candidates(&segments, &name_refs, year);
    let _ = tx.send(Progress::Message(format!(
        "Found {} candidate values locally",
        candidates.len()
    )));
    let mut decisions = Metered {
        session,
        item,
        run,
        inner: me_agent::typesafe::TypeSafe::configured().map_err(|f| f.message)?,
        tx,
    };
    let mut requests = 0u32;
    let mut tokens = 0u64;
    let fail = |f: ImportFailure| {
        let _ = vault_call(session, |v| v.record_import_failure(item, run, &f));
        let _ = tx.send(Progress::Failure(f.clone()));
        f.message
    };

    step(tx, session, item, run, ImportStage::Interpreting, false);
    let classify_version = pipeline::classify_version(&anchors);
    let cached = vault_call(session, |v| {
        v.stage_output(&fingerprint, pipeline::CLASSIFY_STAGE, &classify_version)
    })?
    .and_then(|v| serde_json::from_value::<Classification>(v).ok());
    let classification = match cached {
        Some(c) => c,
        None => {
            let c = pipeline::classify(document, &anchors, &candidates, &mut decisions, cancel)
                .map_err(fail)?;
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
    let tier = vault_call(session, |v| {
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
    step(tx, session, item, run, ImportStage::Context, true);
    let Some(kind) = classification
        .eager_type()
        .filter(|_| tier == me_core::Tier::Eager)
    else {
        // Lazy: classified and searchable; extracted when a question needs it.
        for stage in [ImportStage::Extracting, ImportStage::Verifying] {
            step(tx, session, item, run, stage, true);
        }
        if let Some(batch) = &batch {
            let _ = vault_call(session, |v| {
                v.record_batch_usage(batch, requests, tokens, 0)
            });
        }
        return Ok(GraphRun {
            assertions: 0,
            checks: 0,
            eager: false,
            family: classification.family,
        });
    };

    step(tx, session, item, run, ImportStage::Extracting, false);
    let extract_version = pipeline::extract_version(kind.key, classification.subject.as_deref());
    let cached = vault_call(session, |v| {
        v.stage_output(&fingerprint, pipeline::EXTRACT_STAGE, &extract_version)
    })?
    .and_then(|v| serde_json::from_value::<Extraction>(v).ok());
    let extraction = match cached {
        Some(e) => e,
        None => {
            let mut e = pipeline::extract(
                document,
                kind,
                &classification,
                &candidates,
                threshold,
                &mut decisions,
                cancel,
            )
            .map_err(fail)?;
            // Gap fill: only for missing required values, within the batch allowance.
            let allowed = !e.missing_required.is_empty()
                && codex_ready
                && match &batch {
                    Some(b) => vault_call(session, |v| v.reserve_batch_openai_call(b))?,
                    None => true,
                };
            if allowed && !cancel.load(Ordering::SeqCst) {
                let _ = tx.send(Progress::Message(format!(
                    "Asking the reading model for {} missing value(s)",
                    e.missing_required.len()
                )));
                let mut bytes = 0;
                let section: Vec<me_core::SourceText> = texts
                    .iter()
                    .take_while(|t| {
                        bytes += t.text.len();
                        bytes <= GAP_FILL_BYTES || bytes == t.text.len()
                    })
                    .cloned()
                    .collect();
                let input = me_core::ExtractionInput {
                    run_id: uuid::Uuid::new_v4().to_string(),
                    source_id: source.clone(),
                    title: title.clone(),
                    segments: section,
                };
                let report = me_agent::codex::extract_document(
                    codex_home,
                    &input,
                    &me_agent::guides::ReadingGuide::general(),
                    cancel.clone(),
                    |event| {
                        if matches!(
                            event,
                            Progress::Message(_)
                                | Progress::Failure(_)
                                | Progress::SetupRequired(_)
                        ) {
                            let _ = tx.send(event);
                        }
                    },
                    &mut GapCheckpoints { session, item, run },
                );
                if let Ok(report) = report {
                    let proposed = gap_candidates(&report, &texts, &name_refs, year);
                    pipeline::verify_gap_fill(
                        document,
                        kind,
                        &proposed,
                        &mut e,
                        threshold,
                        &mut decisions,
                        cancel,
                    )
                    .map_err(fail)?;
                }
            }
            requests += e.usage.requests;
            tokens += e.usage.input_tokens;
            let value = serde_json::to_value(&e).map_err(|_| "Invalid extraction.")?;
            let model = e.usage.models.first().cloned();
            vault_call(session, |v| {
                v.save_stage_output(
                    &fingerprint,
                    pipeline::EXTRACT_STAGE,
                    &extract_version,
                    &value,
                    model.as_deref(),
                    Some(e.usage.input_tokens),
                )
            })?;
            e
        }
    };
    step(tx, session, item, run, ImportStage::Extracting, true);
    if cancel.load(Ordering::SeqCst) {
        return Err("Analysis stopped. Saved steps are kept.".into());
    }
    step(tx, session, item, run, ImportStage::Verifying, false);
    let outcome = vault_call(session, |v| {
        v.apply_document_graph(&source, &extraction.graph)
    })?;
    step(tx, session, item, run, ImportStage::Verifying, true);
    if let Some(batch) = &batch {
        let _ = vault_call(session, |v| {
            v.record_batch_usage(batch, requests, tokens, 0)
        });
    }
    Ok(GraphRun {
        assertions: outcome.assertions,
        checks: outcome.checks,
        eager: true,
        family: classification.family,
    })
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
