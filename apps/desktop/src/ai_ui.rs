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
    warning: Option<String>,
    graph: Option<super::read_import::ReadRun>,
}

/// Durable reader checkpoints and step results. `pipeline` namespaces them by
/// reader version and guide, so a section read with another guide is read again.
pub(super) struct VaultCheckpoints {
    pub(super) session: Arc<Mutex<Option<Vault>>>,
    pub(super) pipeline: String,
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
            .extraction_checkpoint(input, &self.pipeline)
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
            .save_extraction_checkpoint(input, &self.pipeline, output)
            .map_err(core_error)
    }
    fn load_step(
        &mut self,
        input: &me_core::ExtractionInput,
        step: &str,
    ) -> Result<Option<serde_json::Value>, String> {
        self.session
            .lock()
            .map_err(|_| "Vault unavailable.")?
            .as_ref()
            .ok_or("Vault locked.")?
            .import_step_cache(input, &self.pipeline, step)
            .map_err(core_error)
    }
    fn save_step(
        &mut self,
        input: &me_core::ExtractionInput,
        step: &str,
        output: &serde_json::Value,
    ) -> Result<(), String> {
        self.session
            .lock()
            .map_err(|_| "Vault unavailable.")?
            .as_mut()
            .ok_or("Vault locked.")?
            .save_import_step_cache(input, &self.pipeline, step, output)
            .map_err(core_error)
    }
    fn reserve(
        &mut self,
        input: &me_core::ExtractionInput,
        provider: me_core::ImportProvider,
    ) -> Result<String, me_core::ImportFailure> {
        reserve_paid_call(&self.session, |v| v.reserve_import_request(input, provider))
    }
}

/// Reserves one paid call durably before it is sent, or checks that one may be.
/// A refusal is reported the same way on every path: the file's or its import's
/// allowance stop as a budget failure of that kind, a paused queue as a quota
/// failure, anything else as a storage failure.
pub(super) fn reserve_paid_call<T>(
    session: &Arc<Mutex<Option<Vault>>>,
    reserve: impl FnOnce(&mut Vault) -> me_core::Result<T>,
) -> Result<T, me_core::ImportFailure> {
    use me_core::{ImportErrorKind as Kind, ImportFailure, ImportProvider};
    let storage = || {
        ImportFailure::new(
            ImportProvider::Local,
            Kind::Storage,
            "The vault is unavailable. Saved work is kept.",
        )
    };
    let mut guard = session.lock().map_err(|_| storage())?;
    let v = guard.as_mut().ok_or_else(storage)?;
    reserve(v).map_err(|e| ImportFailure::refused_reservation(&e))
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
        self.document_read = None;
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
                    vault.document_read(item)?,
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
                        Ok((proposals, read, questions, error, warning)) => {
                            this.questions = questions;
                            this.document_warning = warning;
                            this.proposals = proposals;
                            this.document_read = Some(read);
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
        if !self.ai_ready()
            || self.busy
            || self.active_imports.len() >= import_ui::MAX_ACTIVE_IMPORTS
            || self.import_pause.is_some()
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
                        // Reduce once the map stage is idle: equivalence and alignment.
                        if this.active_imports.is_empty() {
                            this.run_reduce(cx);
                        }
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
            || (with_ai && !self.codex_ready)
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
                        "Unlock your vault first."
                    } else if with_ai && !self.codex_ready {
                        "ChatGPT isn't ready. Check the connection in Settings, or try again shortly."
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
        let mut active = ActiveImport::new(cancel.clone(), automatic);
        if let Some(job) = self.import_jobs.iter().find(|j| j.item == item) {
            active.steps = job.steps;
        }
        self.active_imports.insert(item, active);
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
                        let email = document.extension.eq_ignore_ascii_case("eml");
                        let result = me_documents::extract_with_progress(
                            &document.extension,
                            &document.bytes,
                            &cancel,
                            &mut |page, total| {
                                // Attachment page totals cover only one attachment, not
                                // the whole email. Keep its bar indeterminate until saved.
                                let (completed, known_total) =
                                    if email { (0, 0) } else { (page, total) };
                                if with_ai
                                    && let Ok(mut guard) = session.lock()
                                    && let Some(v) = guard.as_mut()
                                {
                                    let _ = v.update_import_progress(
                                        item,
                                        &run,
                                        me_core::ImportStage::Normalizing,
                                        completed,
                                        known_total,
                                    );
                                }
                                let _ = tx.send(me_agent::codex::Progress::Units {
                                    current: completed,
                                    total: known_total,
                                });
                                let _ = tx.send(me_agent::codex::Progress::Message(if email {
                                    format!("Reading an email attachment · {page} / {total} pages")
                                } else {
                                    format!("Read {page} of {total} pages")
                                }));
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
                            warning: None,
                            graph: None,
                        });
                    }
                    session
                        .lock()
                        .map_err(|_| "Vault unavailable.")?
                        .as_mut()
                        .ok_or("Vault locked.")?
                        .update_import_progress(item, &run, me_core::ImportStage::Normalizing, 1, 1)
                        .map_err(core_error)?;
                    let _ = tx.send(me_agent::codex::Progress::Step {
                        stage: me_core::ImportStage::Normalizing,
                        current: 1,
                        total: 1,
                    });
                    // One pipeline for automatic and manual runs: classify, read the
                    // whole document with its guide, sweep, type, verify, resolve.
                    stage = "read.pipeline";
                    record(
                        stage,
                        &[F::Count("item", item), F::Flag("automatic", automatic)],
                    );
                    // A failure is already recorded: the provider's own, or a local one.
                    let read =
                        super::read_import::run_read(&session, item, &run, &cancel, &tx, &home)?;
                    let guard = session.lock().map_err(|_| "Vault unavailable.")?;
                    let v = guard.as_ref().ok_or("Vault locked.")?;
                    Ok(DocumentResult {
                        collection: v.collection("", false).map_err(core_error)?,
                        proposals: v.proposals(item).map_err(core_error)?,
                        warning: (read.outcome.checks > 0).then(|| {
                            format!(
                                "{} worth a quick look in Review.",
                                if read.outcome.checks == 1 {
                                    "1 value is".to_owned()
                                } else {
                                    format!("{} values are", read.outcome.checks)
                                }
                            )
                        }),
                        graph: Some(read),
                    })
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
                        .finish_import_evaluation(
                            item,
                            &run,
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
                            me_agent::codex::Progress::Step {
                                stage,
                                current,
                                total,
                            } => {
                                if let Some(active) = this.active_imports.get_mut(&item)
                                    && let Some(index) = stage.index()
                                {
                                    active.steps[index] = me_core::StepProgress { current, total };
                                }
                            }
                            me_agent::codex::Progress::Failure(failure) => {
                                if failure.kind.pauses_queue() {
                                    this.import_pause = Some(failure.message.clone());
                                    for (other, active) in &this.active_imports {
                                        if *other != item {
                                            active.cancel.store(true, Ordering::SeqCst);
                                        }
                                    }
                                }
                            }
                            me_agent::codex::Progress::Request { .. }
                            | me_agent::codex::Progress::Usage { .. } => this.refresh_imports(cx),
                            me_agent::codex::Progress::Stage(stage) => {
                                if let Some(active) = this.active_imports.get_mut(&item) {
                                    active.stage = stage;
                                }
                            }
                            me_agent::codex::Progress::Units { current, total } => {
                                if let Some(active) = this.active_imports.get_mut(&item) {
                                    active.current = current;
                                    active.total = total;
                                    active.steps[0] = me_core::StepProgress { current, total };
                                }
                            }
                            me_agent::codex::Progress::Message(s) => {
                                if let Some(active) = this.active_imports.get_mut(&item) {
                                    active.message = s.clone();
                                }
                                if this.ai_item == Some(item) {
                                    this.ai_message = Some(s);
                                }
                            }
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
                    let _ = this.update(cx, |this, cx| {
                        if this.generation == generation {
                            cx.notify();
                        }
                    });
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
                        warning,
                        graph,
                    })) => {
                        if this.search.read(cx).content.is_empty() && !this.pinned_only {
                            this.collection = collection;
                        }
                        this.ai_message = Some(match &graph {
                            Some(r) => format!(
                                "{} values read · {} in your profile",
                                r.outcome.values, r.outcome.in_profile
                            ),
                            None => "File is searchable. Ready for AI analysis.".into(),
                        });
                        if graph.is_some() {
                            this.refresh_graph(cx);
                        }
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
