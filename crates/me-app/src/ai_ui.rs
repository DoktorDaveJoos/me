use super::*;
use me_diagnostics::{Field as F, record};
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) fn core_error(error: me_core::Error) -> String {
    let code = match &error {
        me_core::Error::Io(_) => "io",
        me_core::Error::Database(_) => "database",
        me_core::Error::Authentication => "authentication",
        me_core::Error::Format => "format",
        me_core::Error::InUse => "in_use",
        me_core::Error::Validation(reason) => {
            record("vault.validation", &[F::Label("reason", reason)]);
            "validation"
        }
    };
    record("vault.failed", &[F::Label("code", code)]);
    error.to_string()
}

struct DocumentResult {
    collection: Collection,
    proposals: Vec<me_core::Proposal>,
    fact_count: Option<usize>,
    warning: Option<String>,
}

struct VaultCheckpoints {
    session: Arc<Mutex<Option<Vault>>>,
}
impl me_agent::codex::Checkpoints for VaultCheckpoints {
    fn load(
        &mut self,
        input: &me_core::ExtractionInput,
    ) -> Result<Option<me_core::ExtractionOutput>, String> {
        self.session
            .lock()
            .map_err(|_| "Vault unavailable.")?
            .as_ref()
            .ok_or("Vault locked.")?
            .extraction_checkpoint(input, me_agent::codex::PIPELINE)
            .map_err(core_error)
    }
    fn save(
        &mut self,
        input: &me_core::ExtractionInput,
        output: &me_core::ExtractionOutput,
    ) -> Result<(), String> {
        self.session
            .lock()
            .map_err(|_| "Vault unavailable.")?
            .as_mut()
            .ok_or("Vault locked.")?
            .save_extraction_checkpoint(input, me_agent::codex::PIPELINE, output)
            .map_err(core_error)
    }
}

impl MeApp {
    pub(super) fn stop_ai(&mut self) {
        for active in self.active_imports.values() {
            active.cancel.store(true, Ordering::SeqCst);
        }
    }
    pub(super) fn stop_bridge(&mut self, cx: &mut Context<Self>) {
        if let Some(bridge) = self.bridge.take() {
            bridge.revoke();
            cx.background_executor()
                .spawn(async move { drop(bridge) })
                .detach();
        }
    }
    pub(super) fn toggle_codex(&mut self, cx: &mut Context<Self>) {
        if !self.app_ready() || self.busy {
            return;
        }
        if self.bridge.is_some() {
            self.stop_bridge(cx);
            self.notice = Some("Codex-Lesezugriff beendet.".into());
            cx.notify();
            return;
        }
        let Some(root) = self.root.clone() else {
            return;
        };
        self.busy = true;
        let session = self.session.clone();
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            let scope = session
                .lock()
                .map_err(|_| "Vault unavailable.".to_string())?
                .as_ref()
                .ok_or("Vault locked.")?
                .shareable_scope()
                .map_err(core_error)?;
            me_agent::BridgeServer::start(&root, session, scope)
                .map_err(|_| "Couldn't start connected access.".to_string())
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    if let Ok(bridge) = result {
                        cx.background_executor()
                            .spawn(async move { drop(bridge) })
                            .detach();
                    }
                    return;
                }
                this.busy = false;
                match result {
                    Ok(bridge) => {
                        this.bridge = Some(bridge);
                        this.notice = Some("Connected access is on for this session.".into());
                    }
                    Err(e) => this.notice = Some(e),
                }
                this.kick_auto_queue(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn load_proposals(&mut self, item: u64, cx: &mut Context<Self>) {
        self.proposals.clear();
        self.questions.clear();
        self.question_edit = None;
        self.question_error = None;
        self.proposals_generation += 1;
        let proposals_generation = self.proposals_generation;
        self.document_error = None;
        self.document_warning = None;
        let session = self.session.clone();
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            (|| -> me_core::Result<_> {
                let guard = session.lock().map_err(|_| me_core::Error::Format)?;
                let vault = guard
                    .as_ref()
                    .ok_or(me_core::Error::Validation("Vault locked."))?;
                Ok((
                    vault.proposals(item)?,
                    vault.review_questions(item)?,
                    vault.evaluation_error(item)?,
                    vault.evaluation_warning(item)?,
                ))
            })()
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation
                    && this.document_open == Some(item)
                    && this.proposals_generation == proposals_generation
                {
                    match result {
                        Ok((proposals, questions, error, warning)) => {
                            this.questions = questions;
                            this.document_warning = warning;
                            this.proposals = proposals;
                            this.document_error = error;
                        }
                        Err(error) => {
                            record("document.details.failed", &[F::Count("item", item)]);
                            this.document_error = Some(error.to_string());
                        }
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }
    pub(super) fn kick_auto_queue(&mut self, cx: &mut Context<Self>) {
        if !self.app_ready()
            || self.busy
            || self.active_imports.len() >= import_ui::MAX_ACTIVE_IMPORTS
            || self.auto_queue_check
            || self.settings_saving
            || !self.settings.automatic_evaluation
        {
            return;
        }
        record("queue.check", &[]);
        self.auto_queue_check = true;
        let session = self.session.clone();
        let generation = self.generation;
        let active = self.active_imports.keys().copied().collect::<Vec<_>>();
        let task = cx.background_executor().spawn(async move {
            session
                .lock()
                .map_err(|_| me_core::Error::Format)?
                .as_ref()
                .ok_or(me_core::Error::Validation("Vault locked."))?
                .next_automatic_document_excluding(&active)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.auto_queue_check = false;
                match result {
                    Ok(Some(item))
                        if this.settings.automatic_evaluation && !this.settings_saving =>
                    {
                        if this.active_imports.contains_key(&item) {
                            this.kick_auto_queue(cx);
                        } else {
                            this.start_document(item, true, true, cx);
                        }
                    }
                    Err(error) => {
                        record("queue.failed", &[]);
                        this.ai_failure = Some((None, error.to_string()));
                        this.notice = Some(error.to_string());
                        cx.notify();
                    }
                    _ => {
                        record("queue.idle", &[]);
                    }
                }
            });
        })
        .detach();
    }

    pub(super) fn run_ai(&mut self, item: u64, cx: &mut Context<Self>) {
        self.run_document(item, true, cx);
    }
    pub(super) fn run_document(&mut self, item: u64, with_ai: bool, cx: &mut Context<Self>) {
        self.start_document(item, with_ai, false, cx);
    }
    pub(super) fn start_document(
        &mut self,
        item: u64,
        with_ai: bool,
        automatic: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.app_ready()
            || self.busy
            || self.active_imports.len() >= import_ui::MAX_ACTIVE_IMPORTS
            || self.active_imports.contains_key(&item)
        {
            record(
                "document.deferred",
                &[
                    F::Flag("ready", self.app_ready()),
                    F::Flag("busy", self.busy),
                    F::Flag("processing", !self.active_imports.is_empty()),
                    F::Flag("automatic", automatic),
                ],
            );
            if !automatic && !(self.active_imports.contains_key(&item)) {
                self.document_error = Some(
                    if !self.app_ready() {
                        "Connect ChatGPT first."
                    } else {
                        "Another file is being read. Try again shortly."
                    }
                    .into(),
                );
                cx.notify();
            }
            return;
        }
        let Some(home) = self
            .root
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.join("codex-inbox"))
        else {
            record("document.no_codex_home", &[]);
            self.document_error = Some("Connection settings are unavailable.".into());
            cx.notify();
            return;
        };
        record(
            "document.started",
            &[
                F::Count("item", item),
                F::Flag("automatic", automatic),
                F::Flag("with_ai", with_ai),
            ],
        );
        if self
            .ai_failure
            .as_ref()
            .is_some_and(|(failed, _)| *failed == Some(item))
        {
            self.ai_failure = None;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.active_imports
            .insert(item, ActiveImport::new(cancel.clone(), automatic));
        self.ai_item = Some(item);
        if self.document_open == Some(item) {
            self.document_error = None;
            self.document_warning = None;
        }
        self.ai_message = Some("Reading your file…".into());
        self.error = None;
        let session = self.session.clone();
        let generation = self.generation;
        let (tx, rx) = std::sync::mpsc::channel();
        let run = uuid::Uuid::new_v4().to_string();
        let task = cx.background_executor().spawn(async move {
            let started = std::time::Instant::now();
            let mut stage = "evaluation.begin";
            let outcome = (|| -> Result<Option<DocumentResult>, String> {
                if with_ai {
                    let started = session
                        .lock()
                        .map_err(|_| "Vault unavailable.")?
                        .as_mut()
                        .ok_or("Vault locked.")?
                        .begin_evaluation(item, automatic)
                        .map_err(core_error)?;
                    if !started {
                        return Ok(None);
                    }
                    session
                        .lock()
                        .map_err(|_| "Vault unavailable.")?
                        .as_mut()
                        .ok_or("Vault locked.")?
                        .begin_import_progress(item, &run)
                        .map_err(core_error)?;
                }
                let result = (|| -> Result<DocumentResult, String> {
                    stage = "document.read";
                    record(stage, &[F::Count("item", item)]);
                    let document = {
                        let mut guard = session.lock().map_err(|_| "Vault unavailable.")?;
                        guard
                            .as_mut()
                            .ok_or("Vault locked.")?
                            .begin_document_processing(item)
                            .map_err(core_error)?
                    };
                    if let Some(document) = document {
                        stage = "document.extract_text";
                        record(
                            stage,
                            &[
                                F::Count("item", item),
                                F::Count("bytes", document.bytes.len() as u64),
                            ],
                        );
                        let result = me_documents::extract_with_progress(
                            &document.extension,
                            &document.bytes,
                            &cancel,
                            &mut |page, total| {
                                if with_ai
                                    && let Ok(mut guard) = session.lock()
                                    && let Some(v) = guard.as_mut()
                                {
                                    let _ = v.update_import_progress(
                                        item,
                                        &run,
                                        me_core::ImportStage::Normalizing,
                                        page,
                                        total,
                                    );
                                }
                                let _ = tx.send(me_agent::codex::Progress::Units {
                                    current: page,
                                    total,
                                });
                                let _ = tx.send(me_agent::codex::Progress::Message(format!(
                                    "Reading page {page} of {total}…"
                                )));
                            },
                        );
                        let mut guard = session.lock().map_err(|_| "Vault unavailable.")?;
                        let v = guard.as_mut().ok_or("Vault locked.")?;
                        let outcome = if cancel.load(Ordering::SeqCst) {
                            Err("File processing stopped.".into())
                        } else {
                            result.and_then(|parts| {
                                stage = "document.index";
                                record(
                                    stage,
                                    &[
                                        F::Count("item", item),
                                        F::Count("parts", parts.len() as u64),
                                        F::Count(
                                            "text_bytes",
                                            parts.iter().map(|p| p.text.len() as u64).sum(),
                                        ),
                                    ],
                                );
                                v.finish_document_processing(&document, &parts)
                                    .map_err(core_error)
                            })
                        };
                        if let Err(error) = outcome {
                            let _ = v.fail_document_processing(&document);
                            return Err(error);
                        }
                    }
                    if cancel.load(Ordering::SeqCst) {
                        return Err("File processing stopped.".into());
                    }
                    if !with_ai {
                        let guard = session.lock().map_err(|_| "Vault unavailable.")?;
                        let v = guard.as_ref().ok_or("Vault locked.")?;
                        return Ok(DocumentResult {
                            collection: v.collection("", false).map_err(core_error)?,
                            proposals: v.proposals(item).map_err(core_error)?,
                            fact_count: None,
                            warning: None,
                        });
                    }
                    let _ = tx.send(me_agent::codex::Progress::Units {
                        current: 0,
                        total: 0,
                    });
                    let _ = tx.send(me_agent::codex::Progress::Stage(
                        me_core::ImportStage::Interpreting,
                    ));
                    stage = "ai.prepare";
                    record(stage, &[F::Count("item", item)]);
                    let input = {
                        let mut guard = session.lock().map_err(|_| "Vault unavailable.")?;
                        let v = guard.as_mut().ok_or("Vault locked.")?;
                        v.prepare_extraction(item, me_agent::codex::INBOX_MODEL)
                            .map_err(core_error)?
                    };
                    stage = "ai.provider";
                    record(stage, &[F::Count("item", item)]);
                    let mut units = (0, 0);
                    let mut current_stage = me_core::ImportStage::Interpreting;
                    let mut progress_error = None;
                    let result = me_agent::codex::extract_document(
                        &home,
                        &input,
                        cancel.clone(),
                        |event| {
                            match &event {
                                me_agent::codex::Progress::Stage(stage) => current_stage = *stage,
                                me_agent::codex::Progress::Units { current, total } => {
                                    units = (*current, *total)
                                }
                                _ => {}
                            }
                            if matches!(
                                event,
                                me_agent::codex::Progress::Stage(_)
                                    | me_agent::codex::Progress::Units { .. }
                            ) {
                                let saved = session
                                    .lock()
                                    .map_err(|_| "Vault unavailable.".to_string())
                                    .and_then(|mut guard| {
                                        guard
                                            .as_mut()
                                            .ok_or("Vault locked.".to_string())?
                                            .update_import_progress(
                                                item,
                                                &run,
                                                current_stage,
                                                units.0,
                                                units.1,
                                            )
                                            .map_err(core_error)
                                    });
                                if let Err(error) = saved {
                                    progress_error = Some(error);
                                    cancel.store(true, Ordering::SeqCst);
                                }
                            }
                            let _ = tx.send(event);
                        },
                        &mut VaultCheckpoints {
                            session: session.clone(),
                        },
                    );
                    let mut warning = None;
                    let mut guard = session.lock().map_err(|_| "Vault unavailable.")?;
                    let v = guard.as_mut().ok_or("Vault locked.")?;
                    let outcome = if let Some(error) = progress_error {
                        Err(error)
                    } else if cancel.load(Ordering::SeqCst) {
                        Err(
                            "Analysis stopped. Completed sections are saved; retry to continue."
                                .into(),
                        )
                    } else {
                        result.and_then(|report| {
                            let out = report.output;
                            stage = "ai.validate_and_save";
                            record(
                                stage,
                                &[
                                    F::Count("item", item),
                                    F::Count("facts", out.facts.len() as u64),
                                ],
                            );
                            let _ = tx.send(me_agent::codex::Progress::Stage(
                                me_core::ImportStage::Verifying,
                            ));
                            let count = v
                                .finish_extraction_with_questions(&input, out, report.rejected)
                                .map_err(core_error)?;
                            warning = me_core::question_warning(
                                v.review_questions(item).map_err(core_error)?.len(),
                            );
                            Ok(count)
                        })
                    };
                    match outcome {
                        Ok(_) => {
                            let proposals = v.proposals(item).map_err(core_error)?;
                            Ok(DocumentResult {
                                collection: v.collection("", false).map_err(core_error)?,
                                fact_count: Some(proposals.len()),
                                proposals,
                                warning,
                            })
                        }
                        Err(e) => {
                            let _ = v.fail_extraction(&input.run_id);
                            Err(e)
                        }
                    }
                })();
                if with_ai
                    && let Ok(mut guard) = session.lock()
                    && let Some(vault) = guard.as_mut()
                {
                    if result.is_ok() {
                        vault
                            .update_import_progress(
                                item,
                                &run,
                                me_core::ImportStage::Complete,
                                0,
                                0,
                            )
                            .map_err(core_error)?;
                    }
                    vault
                        .finish_evaluation_with_warning(
                            item,
                            result.as_ref().err().map(String::as_str),
                            result.as_ref().ok().and_then(|r| r.warning.as_deref()),
                        )
                        .map_err(core_error)?;
                }
                result.map(Some)
            })();
            record(
                "document.finished",
                &[
                    F::Count("item", item),
                    F::Label("stage", stage),
                    F::Flag("ok", outcome.is_ok()),
                    F::Flag("cancelled", cancel.load(Ordering::SeqCst)),
                    F::Count("duration_ms", started.elapsed().as_millis() as u64),
                ],
            );
            outcome
        });
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let mut heartbeat = std::time::Instant::now();
            loop {
                let mut disconnected = false;
                loop {
                    let progress = match rx.try_recv() {
                        Ok(p) => p,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            disconnected = true;
                            break;
                        }
                        Err(_) => break,
                    };
                    let _ = this.update(cx, |this, cx| {
                        if this.generation != generation {
                            return;
                        }
                        match progress {
                            me_agent::codex::Progress::Stage(stage) => {
                                if let Some(active) = this.active_imports.get_mut(&item) { active.stage = stage; }
                            }
                            me_agent::codex::Progress::Units {current,total} => {
                                if let Some(active) = this.active_imports.get_mut(&item) { active.current = current; active.total = total; }
                            }
                            me_agent::codex::Progress::Message(s) => {
                                if let Some(active) = this.active_imports.get_mut(&item) { active.message = s.clone(); }
                                if this.ai_item == Some(item) { this.ai_message = Some(s); }
                            },
                            me_agent::codex::Progress::SetupRequired(issue) => {
                                this.require_codex_setup(issue, cx)
                            }
                            me_agent::codex::Progress::LoginUrl(url) => {
                                cx.open_url(&url);
                                this.ai_message =
                                    Some("Sign in to ChatGPT in your browser to continue.".into());
                            }
                        }
                        cx.notify();
                    });
                }
                if disconnected {
                    break;
                }
                if heartbeat.elapsed() >= std::time::Duration::from_secs(1) {
                    let _ = this.update(cx, |this,cx| {if this.generation == generation {cx.notify();}});
                    heartbeat = std::time::Instant::now();
                }
                executor.timer(std::time::Duration::from_millis(150)).await;
            }
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.active_imports.remove(&item);
                this.ai_item = Some(item);
                match result {
                    Ok(Some(DocumentResult {
                        collection,
                        proposals,
                        fact_count: count,
                        warning,
                    })) => {
                        if this.search.read(cx).content.is_empty() && !this.pinned_only {
                            this.collection = collection;
                        }
                        this.ai_message = Some(match count {
                            None => "File is searchable. Ready for AI analysis.".into(),
                            Some(0) => "No supported details found. Check the original if you expected more.".into(),
                            Some(count) => format!("{count} details ready for review."),
                        });
                        if let Some(warning) = warning {
                            this.ai_message = Some(format!(
                                "{} {warning}",
                                this.ai_message.as_deref().unwrap_or_default()
                            ));
                            this.ai_warning = Some((item, warning));
                        } else if this.ai_warning.as_ref().is_some_and(|(id, _)| *id == item) {
                            this.ai_warning = None;
                        }
                        if this.document_open == Some(item) {
                            this.proposals = proposals;
                            this.load_proposals(item, cx);
                        }
                    }
                    Ok(None) => {
                        this.ai_message = None;
                    }
                    Err(e) => {
                        this.ai_failure = Some((Some(item), e.clone()));
                        if this.document_open == Some(item) {
                            this.document_error = Some(e.clone());
                        }
                        this.ai_message = Some(e);
                    }
                }
                this.notice = None;
                this.organization_attempted = false;
                if this.page == Page::Browser {
                    this.organize_files(cx);
                }
                this.refresh_search(cx);
                this.refresh_imports(cx);
                this.kick_auto_queue(cx);
                cx.notify();
            });
        })
        .detach();
        self.refresh_imports(cx);
        self.kick_auto_queue(cx);
        self.refresh_search(cx);
        cx.notify();
    }
}
