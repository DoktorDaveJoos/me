use super::*;
use gpui::SharedString;
use me_core::{ImportJob, ImportStage};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};

pub(super) const MAX_ACTIVE_IMPORTS: usize = 2;
pub(super) struct ActiveImport {
    pub cancel: Arc<AtomicBool>,
    pub automatic: bool,
    pub stage: ImportStage,
    pub steps: [me_core::StepProgress; 5],
    pub message: String,
    pub current: u32,
    pub total: u32,
    pub started: Instant,
}
impl ActiveImport {
    pub fn new(cancel: Arc<AtomicBool>, automatic: bool) -> Self {
        Self {
            cancel,
            automatic,
            stage: ImportStage::Normalizing,
            steps: [me_core::StepProgress::default(); 5],
            message: "Reading the original and recognizing text…".into(),
            current: 0,
            total: 0,
            started: Instant::now(),
        }
    }
}
#[derive(Clone)]
pub(super) struct PendingFile {
    pub path: PathBuf,
    pub selected: bool,
    pub detail: String,
    pub error: Option<String>,
}
impl PendingFile {
    fn inspect(path: PathBuf) -> Self {
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let mut detail = String::new();
        let error = match std::fs::metadata(&path) {
            Ok(m) if !m.is_file() => {
                Some("Choose individual files; folders are not imported.".into())
            }
            Ok(m) if m.len() > 64 * 1024 * 1024 => Some("Exceeds the 64 MiB file limit.".into()),
            Ok(m) if m.len() == 0 => Some("This file is empty.".into()),
            Ok(m) => {
                detail = format!(
                    "{} · {} · {}",
                    extension.to_uppercase(),
                    if m.len() < 1024 {
                        format!("{} bytes", m.len())
                    } else {
                        format!("{:.1} KiB", m.len() as f64 / 1024.)
                    },
                    if extension == "1pux" {
                        "Opens the separate password import"
                    } else if me_core::processable_document(&extension) {
                        "Ready to read"
                    } else {
                        "Original only; convert to PDF or text for analysis"
                    }
                );
                if me_core::supported_document(&path) || extension == "1pux" {
                    None
                } else {
                    Some("Unsupported format. Export as PDF, image, EML or text.".into())
                }
            }
            Err(_) => Some("File unavailable. Check access permissions.".into()),
        };
        Self {
            path,
            selected: error.is_none(),
            detail,
            error,
        }
    }
}

impl MeApp {
    pub(super) fn accept_documents(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        // Metadata only. Nothing is imported or released to AI until confirmation.
        let paths = paths.to_vec();
        let generation = self.generation;
        let pending_generation = self.pending_generation;
        self.import_scans += 1;
        let task = cx.background_executor().spawn(async move {
            paths
                .into_iter()
                .map(PendingFile::inspect)
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let files = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation || this.pending_generation != pending_generation {
                    return;
                }
                this.import_scans = this.import_scans.saturating_sub(1);
                for file in files {
                    if !this.pending_imports.iter().any(|p| p.path == file.path) {
                        this.pending_imports.push(file);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn cancel_import_confirmation(&mut self, cx: &mut Context<Self>) {
        self.pending_generation += 1;
        self.import_scans = 0;
        self.pending_imports.clear();
        self.pending_attach_to_search = false;
        cx.notify();
    }
    pub(super) fn confirm_imports(&mut self, cx: &mut Context<Self>) {
        if !self.app_ready() || self.import_scans > 0 || self.intake_active || self.busy {
            return;
        }
        let paths = self
            .pending_imports
            .iter()
            .filter(|p| p.selected && p.error.is_none())
            .map(|p| p.path.clone())
            .collect::<Vec<_>>();
        if paths.is_empty() {
            return;
        }
        if paths.iter().any(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("1pux"))
        }) {
            if paths.len() == 1 {
                self.cancel_import_confirmation(cx);
                self.show_settings = true;
                self.show_onepassword = true;
                self.prepare_onepassword(paths[0].clone(), cx);
            } else {
                self.error = Some(
                    "Select only the 1Password export, or deselect it to import documents.".into(),
                );
                cx.notify();
            }
            return;
        }
        let attach_to_search = self.pending_attach_to_search;
        self.cancel_import_confirmation(cx);
        self.page = if attach_to_search {
            Page::Search
        } else {
            Page::Imports
        };
        self.show_settings = false;
        self.show_add = false;
        self.document_open = None;
        self.intake_active = true;
        self.intake_files = paths.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.intake_cancel = Some(cancel.clone());
        self.error = None;
        self.intake_message = Some(format!("Saving {} originals…", paths.len()));
        let session = self.session.clone();
        let generation = self.generation;
        let total = paths.len();
        let (tx, rx) = std::sync::mpsc::channel();
        cx.background_executor()
            .spawn(async move {
                for (index, path) in paths.into_iter().enumerate() {
                    if cancel.load(Ordering::SeqCst) {
                        break;
                    }
                    // Serialize short vault writes; OCR and model work run outside this lock.
                    let result = session
                        .lock()
                        .map_err(|_| "Vault unavailable.".to_string())
                        .and_then(|mut guard| {
                            let vault = guard.as_mut().ok_or("Vault locked.".to_string())?;
                            let item = vault
                                .import_document(
                                    &path,
                                    &uuid::Uuid::new_v4().to_string(),
                                    me_core::DocumentClass::Unclassified,
                                )
                                .map_err(|e| e.to_string())?;
                            if attach_to_search {
                                vault
                                    .defer_automatic_document(item)
                                    .map_err(|e| e.to_string())?;
                            }
                            Ok(item)
                        });
                    if tx.send((path, result, index + 1)).is_err() {
                        break;
                    }
                }
            })
            .detach();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            loop {
                let mut finished = false;
                loop {
                    match rx.try_recv() {
                        Ok((path, result, count)) => {
                            let _ = this.update(cx, |this, cx| {
                                if this.generation != generation {
                                    return;
                                }
                                this.intake_files.retain(|pending| *pending != path);
                                match result {
                                    Err(error) => this.intake_failures.push((path, error)),
                                    Ok(id) => {
                                        this.intake_failures.retain(|(failed, _)| *failed != path);
                                        if attach_to_search {
                                            this.attachments.push(id);
                                        }
                                    }
                                }
                                this.intake_message =
                                    Some(format!("Saved or checked {count} of {total} files"));
                                this.refresh_imports(cx);
                                this.kick_auto_queue(cx);
                                cx.notify();
                            });
                        }
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            finished = true;
                            break;
                        }
                        Err(_) => break,
                    }
                }
                if finished {
                    break;
                }
                executor.timer(std::time::Duration::from_millis(150)).await;
            }
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.intake_active = false;
                this.intake_cancel = None;
                this.intake_files.clear();
                this.intake_message = None;
                this.organization_attempted = false;
                if attach_to_search {
                    this.inspect_attached_forms(cx);
                }
                this.refresh_imports(cx);
                this.refresh_search(cx);
                this.kick_auto_queue(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn refresh_imports(&mut self, cx: &mut Context<Self>) {
        if !self.unlocked {
            return;
        }
        if self.import_refresh {
            self.import_refresh_pending = true;
            return;
        }
        self.import_refresh = true;
        let generation = self.generation;
        let session = self.session.clone();
        let task = cx.background_executor().spawn(async move {
            let guard = session.lock().map_err(|_| me_core::Error::Format)?;
            let vault = guard.as_ref().ok_or(me_core::Error::Format)?;
            Ok::<_, me_core::Error>((vault.import_jobs()?, vault.import_pause_reason()?))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.import_refresh = false;
                match result {
                    Ok((jobs, pause)) => {
                        this.expanded_imports
                            .retain(|item| jobs.iter().any(|job| job.item == *item));
                        this.import_jobs = jobs;
                        this.import_pause = pause;
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                if std::mem::take(&mut this.import_refresh_pending) {
                    this.refresh_imports(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn import_confirmation(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self
            .pending_imports
            .iter()
            .filter(|p| p.selected && p.error.is_none())
            .count();
        let can_import =
            selected > 0 && self.import_scans == 0 && !self.intake_active && !self.busy;
        self.overlay(cx).child(modal_panel(620.).max_h_full()
            .child(heading("Import these files?"))
            .child(div().type_style(Type::Body).text_color(rgb(MUTED)).child("Check the files below. Only selected files will be added to ME."))
            .child(div().id("import-confirm-files").min_h_0().flex_shrink().max_h(px(250.)).overflow_y_scroll().flex().flex_col().gap(px(space::SM))
                .children(self.pending_imports.iter().enumerate().map(|(index, file)| {
                    let valid = file.error.is_none();
                    div().id(SharedString::from(format!("confirm-file-{index}"))).p(px(space::MD)).border_1().border_color(rgb(if file.selected {FOCUS} else {LINE})).rounded(px(radius::STANDARD)).flex().items_start().gap(px(space::MD))
                        .when(valid, |s| s.cursor_pointer().on_click(cx.listener(move |this,_,_,cx| {
                            if let Some(file) = this.pending_imports.get_mut(index) {file.selected = !file.selected;}
                            cx.notify();
                        })))
                        .child(icon(if file.selected {Icon::Check} else {Icon::Document}, IconSize::Medium, if valid {ACCENT} else {MUTED}))
                        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(space::XS))
                            .child(div().type_style(Type::Body).child(file.path.file_name().unwrap_or_default().to_string_lossy().into_owned()))
                            .child(div().type_style(Type::Caption).text_color(rgb(MUTED)).overflow_hidden().text_ellipsis().child(file.path.display().to_string()))
                            .child(div().type_style(Type::Small).text_color(rgb(if valid {MUTED} else {DANGER})).child(file.error.clone().unwrap_or_else(|| file.detail.clone()))))
                })))
            .when(self.import_scans > 0, |s| s.child(div().type_style(Type::Small).child("Checking file names, formats and sizes…")))
            .child(div().type_style(Type::Small).text_color(rgb(MUTED)).child(if self.pending_attach_to_search {"The selected files will be saved and attached in Search. Form text is analyzed with your connected ChatGPT account."} else if self.settings.automatic_evaluation {"Originals stay in your vault. Extracted content goes to TypeSafe for typed checks and to OpenAI for extraction. Calls are capped per file; results remain suggestions for review."} else {"Originals stay in your vault. Automatic analysis is off; you can start it for each file in Imports."}))
            .when_some(self.error.clone(), |s, e| s.child(div().type_style(Type::Small).text_color(rgb(DANGER)).child(e)))
            .child(div().flex().items_center().justify_between().gap(px(space::LG))
                .child(div().id("cancel-import").type_style(Type::Body).cursor_pointer().on_click(cx.listener(|this,_,_,cx|this.cancel_import_confirmation(cx))).child("Cancel"))
                .child(primary_action().id("confirm-import").opacity(if can_import {1.} else {0.45})
                    .on_click(cx.listener(|this,_,_,cx|this.confirm_imports(cx)))
                    .child(if self.intake_active {"Saving previous files…".into()} else {format!("Import {selected} {}", if selected == 1 {"file"} else {"files"})}))))
    }
    pub(super) fn import_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.active_imports.len();
        let queued = self
            .import_jobs
            .iter()
            .filter(|j| {
                j.state == "queued" && j.processable && !self.active_imports.contains_key(&j.item)
            })
            .count();
        div().id("import-page").flex_1().min_h_0().overflow_y_scroll().px(px(space::PAGE)).pb(px(space::PAGE))
            .child(div().max_w(px(layout::SEARCH_WIDTH)).mx_auto().pt(px(space::PAGE_TOP)).flex().flex_col().gap(px(space::XXL))
                .child(div().flex().items_center().justify_between().gap(px(space::LG))
                    .child(heading("Imports"))
                    .child(primary_action().id("choose-imports").on_click(cx.listener(|this,_,_,cx|this.pick_documents(cx))).child(icon(Icon::Plus,IconSize::Medium,SURFACE)).child("Add files")))
                .child(div().type_style(Type::Body).text_color(rgb(MUTED)).child("Every file, every step. Progress is saved as your documents are understood."))
                .when_some(self.import_pause.clone(),|s,reason|s.child(div().p(px(space::LG)).rounded(px(radius::STANDARD)).bg(rgb(WARNING_SURFACE)).flex().flex_col().gap(px(space::SM))
                    .child(div().type_style(Type::Body).text_color(rgb(WARNING)).child(reason))
                    .child(div().id("resume-import-queue").type_style(Type::Small).text_color(rgb(ACCENT)).cursor_pointer().on_click(cx.listener(|this,_,_,cx|this.resume_import_queue(cx))).child("I have checked the provider · resume queue"))))
                .child(div().flex().gap(px(space::LG)).type_style(Type::Small).text_color(rgb(MUTED))
                    .child(format!("{active} processing"))
                    .child(format!("{queued} queued"))
                    .child(if self.settings.automatic_evaluation {"Automatic reading on"} else {"Automatic reading off"}))
                .when_some(self.intake_message.clone(), |s,m|s.child(div().type_style(Type::Body).text_color(rgb(ACCENT)).child(m)))
                .when(self.import_jobs.is_empty() && !self.intake_active, |s|s.child(div().p(px(space::SECTION)).bg(rgb(SURFACE)).rounded(px(radius::STANDARD)).border_1().border_color(rgb(LINE)).flex().flex_col().gap(px(space::MD))
                    .child(icon(Icon::Upload,IconSize::Large,ACCENT)).child(div().type_style(Type::Section).child("Your next file starts here"))
                    .child(div().type_style(Type::Body).text_color(rgb(MUTED)).child("Letters, insurance documents, emails, PDFs, photos and notes. You choose what gets imported."))))
                .children(self.intake_files.iter().enumerate().map(|(index,path)| div().p(px(space::LG)).bg(rgb(SURFACE)).rounded(px(radius::STANDARD)).border_1().border_color(rgb(LINE)).flex().flex_col().gap(px(space::SM))
                .child(div().type_style(Type::Label).child(path.file_name().unwrap_or_default().to_string_lossy().into_owned()))
                .child(div().type_style(Type::Small).text_color(rgb(MUTED)).child(if index == 0 {"Saving original…"} else {"Waiting to save"}))))
            .children(self.intake_failures.iter().enumerate().map(|(index,(path,error))| {
                    let retry = path.clone();
                    div().p(px(space::LG)).rounded(px(radius::STANDARD)).bg(rgb(DANGER_SURFACE)).flex().flex_col().gap(px(space::SM))
                        .child(div().type_style(Type::Body).child(path.file_name().unwrap_or_default().to_string_lossy().into_owned()))
                        .child(div().type_style(Type::Small).text_color(rgb(DANGER)).child(format!("Not imported · {error}")))
                        .child(div().id(SharedString::from(format!("retry-intake-{index}"))).type_style(Type::Small).text_color(rgb(ACCENT)).cursor_pointer().on_click(cx.listener(move |this,_,_,cx|this.accept_documents(std::slice::from_ref(&retry),cx))).child("Choose for import again"))
                }))
                .when_some(self.error.clone(),|s,e|s.child(div().type_style(Type::Small).text_color(rgb(DANGER)).child(e)))
                .children(self.import_jobs.iter().map(|job|self.import_row(job,cx).into_any_element()))
                .child(div().type_style(Type::Caption).text_color(rgb(MUTED)).child("Bars count completed pages and sections, not elapsed time. TypeSafe guides decisions; OpenAI extracts details. No automatic retries or web research.")))
    }
    fn import_row(&self, job: &ImportJob, cx: &mut Context<Self>) -> impl IntoElement {
        let item = job.item;
        let expanded = self.expanded_imports.contains(&item);
        let active = self.active_imports.get(&item);
        let stage = active.map(|a| a.stage).unwrap_or(job.stage);
        let running = active.is_some();
        let failed = job.state == "failed" && !running;
        let complete = job.state == "done" && !running;
        let steps = active.map(|a| a.steps).unwrap_or(job.steps).map(|p| {
            if complete && p.total == 0 {
                me_core::StepProgress {
                    current: 1,
                    total: 1,
                }
            } else {
                p
            }
        });
        let progress =
            steps.iter().map(|p| p.fraction()).sum::<f32>() / ImportStage::STEPS.len() as f32;
        let activity = active
            .map(|a| a.started.elapsed().as_secs_f32() / 3.)
            .unwrap_or(0.);
        let color = if running {
            ACCENT
        } else if failed {
            DANGER
        } else if complete {
            SUCCESS
        } else {
            MUTED
        };
        let status = if running {
            format!(
                "{} · {}s elapsed",
                stage.label(),
                active.unwrap().started.elapsed().as_secs()
            )
        } else if !job.processable {
            "Original saved · conversion needed".into()
        } else if failed {
            match &job.failure {
                Some(failure) => format!(
                    "Paused at {} · {}",
                    stage.label(),
                    error_label(failure.kind)
                ),
                None => format!("Paused at {}", stage.label()),
            }
        } else if complete {
            format!(
                "Ready · {} suggestions · {} questions",
                job.proposals, job.questions
            )
        } else if job.state == "manual" {
            "Ready when you are".into()
        } else {
            "Queued · original safely saved".into()
        };
        let used = job
            .usage
            .input_tokens
            .saturating_add(job.usage.output_tokens);
        div().p(px(space::XL)).bg(rgb(SURFACE)).rounded(px(radius::STANDARD)).border_1().border_color(rgb(LINE)).flex().flex_col().gap(px(space::LG))
            .child(div().flex().items_start().gap(px(space::MD))
                .child(icon(Icon::Document,IconSize::Large,if running {ACCENT}else{MUTED}))
                .child(div().flex_1().min_w_0().flex().flex_col().gap(px(space::XS))
                    .child(div().type_style(Type::Label).font_weight(font::MEDIUM).child(job.title.clone()))
                    .child(div().type_style(Type::Small).text_color(rgb(color)).child(status)))
                .child(div().id(SharedString::from(format!("import-open-{item}"))).type_style(Type::Small).text_color(rgb(ACCENT)).cursor_pointer()
                    .on_click(cx.listener(move |this,_,_,cx|{this.document_open=Some(item);this.load_proposals(item,cx);cx.notify();})).child("Open")))
            .when(job.processable,|s|s
                .child(div().flex().flex_col().gap(px(space::SM))
                    .child(div().flex().justify_between().type_style(Type::Small)
                        .child("Completed work")
                        .child(div().font_family(font::MONO).text_color(rgb(color)).child(format!("{:.0}%",progress*100.))))
                    .child(progress_bar(Some(progress),color,activity))))
            .child(div().id(SharedString::from(format!("import-details-{item}"))).min_h(px(layout::CONTROL_COMPACT)).rounded(px(radius::STANDARD))
                .flex().items_center().gap(px(space::SM)).type_style(Type::Small).text_color(rgb(ACCENT)).cursor_pointer().hover(|s|s.bg(rgb(HOVER)))
                .on_click(cx.listener(move |this,_,_,cx|{
                    if !this.expanded_imports.remove(&item) { this.expanded_imports.insert(item); }
                    cx.notify();
                }))
                .child(icon(if expanded {Icon::Down}else{Icon::Chevron},IconSize::Small,ACCENT))
                .child(if expanded {"Hide details"} else if failed {"Show error and details"} else {"Show details"}))
            .when(expanded,|s|s
            .when(job.processable,|s|s
                .child(div().flex().flex_col().gap(px(space::SM)).children(ImportStage::STEPS.iter().enumerate().map(|(index,step)|{
                    let count=steps[index];let current=running && *step==stage;
                    let tint=if failed && *step==stage {DANGER} else if count.complete() {SUCCESS} else if current {ACCENT} else {MUTED};
                    let label=if count.complete() {"Done".into()} else if count.total>0 {format!("{} / {}",count.current,count.total)} else if current {"Working".into()} else {"Waiting".into()};
                    div().flex().items_center().gap(px(space::MD))
                        .child(div().w(px(112.)).flex_shrink_0().type_style(Type::Small).text_color(rgb(tint)).child(step.label()))
                        .child(div().flex_1().min_w_0().child(progress_bar(if current && count.total==0 {None}else{Some(count.fraction())},tint,activity)))
                        .child(div().w(px(64.)).text_right().type_style(Type::Caption).font_family(font::MONO).text_color(rgb(tint)).child(label))
                }))))
            .when_some(active.map(|a|a.message.clone()),|s,m|s.child(div().type_style(Type::Small).text_color(rgb(MUTED)).child(m)))
            .when_some(if running {None}else{job.error.clone().or_else(||job.warning.clone())},|s,message|s.child(
                div().p(px(space::MD)).rounded(px(radius::STANDARD)).bg(rgb(if failed {DANGER_SURFACE}else{WARNING_SURFACE})).flex().flex_col().gap(px(space::XS))
                    .when_some(job.failure.as_ref().map(|f|format!("{} · {}",f.provider.label(),error_label(f.kind))),|s,label|s.child(div().type_style(Type::Small).font_weight(font::MEDIUM).text_color(rgb(color)).child(label)))
                    .child(div().type_style(Type::Small).text_color(rgb(if failed {DANGER}else{WARNING})).child(message))
                    .when(failed,|s|s.child(div().type_style(Type::Caption).text_color(rgb(MUTED)).child("Your original and completed steps are saved. Resume continues from saved work.")))))
            .child(div().border_t_1().border_color(rgb(LINE)).pt(px(space::MD)).flex().flex_col().gap(px(space::XS))
                .child(div().type_style(Type::Caption).text_color(rgb(MUTED)).child(format!("OpenAI {} / {} calls · TypeSafe {} / {} calls",job.usage.openai_calls,job.usage.openai_limit,job.usage.typesafe_calls,job.usage.typesafe_limit)))
                .child(div().type_style(Type::Caption).text_color(rgb(MUTED)).child(if job.usage.unreported_calls>0 {format!("{used} reported tokens · usage unreported for {} calls",job.usage.unreported_calls)} else {format!("{used} reported tokens · {} allowance",job.usage.token_limit)})))
            .when(running,|s|s.child(div().id(SharedString::from(format!("stop-import-{item}"))).type_style(Type::Small).text_color(rgb(INK)).cursor_pointer()
                .on_click(cx.listener(move |this,_,_,cx|{if let Some(a)=this.active_imports.get_mut(&item) {a.cancel.store(true,Ordering::SeqCst);a.message="Stopping; keeping completed steps…".into();}cx.notify();})).child("Pause and keep progress")))
            .when(!running && job.processable && matches!(job.state.as_str(),"failed"|"manual"),|s|{
                let allowance=job.usage.exhausted() || job.failure.as_ref().is_some_and(|f|f.kind==me_core::ImportErrorKind::Budget);
                let available=self.active_imports.len()<MAX_ACTIVE_IMPORTS && self.import_pause.is_none();
                s.child(div().id(SharedString::from(format!("retry-import-{item}"))).type_style(Type::Small).text_color(rgb(if available {ACCENT}else{MUTED})).when(available,|s|s.cursor_pointer()
                    .on_click(cx.listener(move |this,_,_,cx|{if allowance {this.extend_import(item,cx);}else{this.run_ai(item,cx);}})))
                    .child(if self.import_pause.is_some() {"Resume the queue above first"} else if !available {"Waiting for an available worker"} else if allowance {"Allow 12 more OpenAI + 24 TypeSafe calls and resume"} else if failed {"Resume saved steps"} else {"Start analysis"}))
            }))
    }
    fn extend_import(&mut self, item: u64, cx: &mut Context<Self>) {
        if self.busy || self.active_imports.contains_key(&item) {
            return;
        }
        self.busy = true;
        let session = self.session.clone();
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            session
                .lock()
                .map_err(|_| me_core::Error::Format)?
                .as_mut()
                .ok_or(me_core::Error::Format)?
                .extend_import_allowance(item)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.busy = false;
                match result {
                    Ok(()) => this.run_ai(item, cx),
                    Err(e) => this.error = Some(e.to_string()),
                }
                this.refresh_imports(cx);
                cx.notify();
            });
        })
        .detach();
    }
    fn resume_import_queue(&mut self, cx: &mut Context<Self>) {
        let session = self.session.clone();
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            session
                .lock()
                .map_err(|_| me_core::Error::Format)?
                .as_mut()
                .ok_or(me_core::Error::Format)?
                .resume_import_queue()
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(()) => {
                        this.import_pause = None;
                        this.kick_auto_queue(cx);
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                this.refresh_imports(cx);
                cx.notify();
            });
        })
        .detach();
    }
}

fn error_label(kind: me_core::ImportErrorKind) -> &'static str {
    use me_core::ImportErrorKind::*;
    match kind {
        Quota => "Usage limit",
        RateLimit => "Rate limit",
        Authentication => "Connection setup",
        Timeout => "Request timed out",
        Connection => "Connection interrupted",
        InvalidOutput => "Response needs attention",
        Budget => "Analysis allowance reached",
        Cancelled => "Paused by you",
        Interrupted => "Interrupted",
        Storage => "Vault or processing error",
        Unprocessable => "Source needs attention",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preflight_does_not_accept_empty_missing_or_directory_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("empty.txt");
        std::fs::write(&empty, "").unwrap();
        for path in [dir.path().to_owned(), empty, dir.path().join("missing.pdf")] {
            let file = PendingFile::inspect(path);
            assert!(!file.selected);
            assert!(file.error.is_some());
        }
    }
}
