//! "Who are you?" setup, the folder dump, the growing constellation and quick
//! checks. Vault work runs on the background executor; the UI only renders state.
use super::*;
use crate::design_system::constellation as sky;
use gpui::{PathBuilder, SharedString, canvas, point};
use me_core::{
    BatchProgress, Constellation, FolderPlan, HouseholdProposal, IdentityAnchors, MergeCandidate,
    QuickCheckGroup, SourceKind,
};
use std::sync::atomic::{AtomicBool, Ordering};

const ROLES: &[(&str, &str)] = &[
    ("partner", "Partner"),
    ("child", "Child"),
    ("parent", "Parent"),
    ("other", "Other"),
];
const SETUP_STEPS: &[&str] = &["You", "Born", "Home", "Files"];

#[derive(Default)]
pub(super) struct GraphState {
    pub anchors: Option<IdentityAnchors>,
    pub checks: Vec<QuickCheckGroup>,
    pub households: Vec<HouseholdProposal>,
    pub merges: Vec<MergeCandidate>,
    pub constellation: Constellation,
    pub batch: Option<BatchProgress>,
    loading: bool,
    reload: bool,
    pub setup_step: usize,
    role: usize,
    setup_busy: bool,
    pub setup_error: Option<String>,
    pub plan: Option<(Vec<PathBuf>, FolderPlan)>,
    planning: bool,
    pub dump_cancel: Option<Arc<AtomicBool>>,
    dump_saved: usize,
    dump_duplicates: usize,
    dump_failed: usize,
    reduce_running: bool,
    reduce_done_for: usize,
    busy: BTreeSet<String>,
    pub lazy_documents: usize,
    deep_query: Option<String>,
    deep_running: bool,
    deep_found: Vec<(u64, String)>,
    deep_error: Option<String>,
}

fn birth_date(input: &str) -> Result<Option<String>, &'static str> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(None);
    }
    let parts: Vec<&str> = input.split(['.', '/', '-']).map(str::trim).collect();
    let (d, m, y) = match parts.as_slice() {
        [y, m, d] if y.len() == 4 => (*d, *m, *y),
        [d, m, y] if y.len() == 4 => (*d, *m, *y),
        _ => return Err("Use a date like 14.03.1988."),
    };
    let (Ok(d), Ok(m), Ok(y)) = (d.parse::<u32>(), m.parse::<u32>(), y.parse::<u32>()) else {
        return Err("Use a date like 14.03.1988.");
    };
    let iso = format!("{y:04}-{m:02}-{d:02}");
    // A calendar-invalid date is rejected by the vault when it is saved.
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return Err("Use a date like 14.03.1988.");
    }
    Ok(Some(iso))
}
fn first_name(name: &str) -> &str {
    name.split_whitespace().next().unwrap_or(name)
}
fn mebibytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GiB", bytes as f64 / (1024. * 1024. * 1024.))
    } else {
        format!("{:.0} MiB", (bytes as f64 / (1024. * 1024.)).max(1.))
    }
}

impl MeApp {
    pub(super) fn identity_setup_visible(&self) -> bool {
        self.graph
            .anchors
            .as_ref()
            .is_some_and(|a| !a.setup_complete)
    }

    /// Loads anchors, quick checks, proposals, the constellation and batch progress.
    pub(super) fn refresh_graph(&mut self, cx: &mut Context<Self>) {
        if !self.app_ready() {
            return;
        }
        if self.graph.loading {
            self.graph.reload = true;
            return;
        }
        self.graph.loading = true;
        let session = self.session.clone();
        let generation = self.generation;
        let batch = self.graph.batch.as_ref().map(|b| b.id.clone());
        let task = cx.background_executor().spawn(async move {
            let guard = session.lock().map_err(|_| me_core::Error::Format)?;
            let v = guard.as_ref().ok_or(me_core::Error::Format)?;
            let batch = match batch {
                Some(b) => Some(v.batch_progress(&b)?),
                None => None,
            };
            let lazy = v.lazy_document_count()?;
            Ok::<_, me_core::Error>((
                lazy,
                v.identity_anchors()?,
                v.quick_checks()?,
                v.household_proposals()?,
                v.merge_proposals()?,
                v.constellation(sky::MAX_NODES)?,
                batch,
            ))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.graph.loading = false;
                if let Ok((lazy, anchors, checks, households, merges, constellation, batch)) =
                    result
                {
                    this.graph.lazy_documents = lazy;
                    let was_unknown = this.graph.anchors.is_none();
                    if was_unknown || this.graph.setup_busy {
                        this.identity_name.update(cx, |i, cx| {
                            if i.content.is_empty() && anchors.self_name != "Ich" {
                                i.set_text(&anchors.self_name, cx);
                            }
                        });
                    }
                    this.graph.anchors = Some(anchors);
                    this.graph.checks = checks;
                    this.graph.households = households;
                    this.graph.merges = merges;
                    this.graph.constellation = constellation;
                    if batch.is_some() {
                        this.graph.batch = batch;
                    }
                }
                if std::mem::take(&mut this.graph.reload) {
                    this.refresh_graph(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn setup_call(
        &mut self,
        cx: &mut Context<Self>,
        work: impl FnOnce(&mut Vault) -> me_core::Result<()> + Send + 'static,
        next: usize,
    ) {
        if self.graph.setup_busy {
            return;
        }
        self.graph.setup_busy = true;
        self.graph.setup_error = None;
        let session = self.session.clone();
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            let mut guard = session.lock().map_err(|_| me_core::Error::Format)?;
            work(guard.as_mut().ok_or(me_core::Error::Format)?)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.graph.setup_busy = false;
                match result {
                    Ok(()) => this.graph.setup_step = next,
                    Err(e) => this.graph.setup_error = Some(e.to_string()),
                }
                this.refresh_graph(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn save_identity_name(&mut self, cx: &mut Context<Self>) {
        let name = self.identity_name.read(cx).content.trim().to_owned();
        if name.is_empty() {
            self.graph.setup_error = Some("Tell ME. your name first.".into());
            cx.notify();
            return;
        }
        let aliases: Vec<String> = self
            .identity_aliases
            .read(cx)
            .content
            .split([',', ';'])
            .map(|a| a.trim().to_owned())
            .filter(|a| !a.is_empty())
            .collect();
        let birth = self
            .graph
            .anchors
            .as_ref()
            .and_then(|a| a.birth_date.clone());
        self.setup_call(
            cx,
            move |v| v.set_identity(&name, &aliases, birth.as_deref()),
            1,
        );
    }
    fn save_birth_date(&mut self, cx: &mut Context<Self>) {
        let birth = match birth_date(&self.identity_birth.read(cx).content) {
            Ok(b) => b,
            Err(message) => {
                self.graph.setup_error = Some(message.into());
                cx.notify();
                return;
            }
        };
        let Some(anchors) = self.graph.anchors.clone() else {
            return;
        };
        self.setup_call(
            cx,
            move |v| v.set_identity(&anchors.self_name, &anchors.aliases, birth.as_deref()),
            2,
        );
    }
    fn add_household(&mut self, cx: &mut Context<Self>) {
        let name = self.household_name.read(cx).content.trim().to_owned();
        if name.is_empty() {
            return;
        }
        let role = ROLES[self.graph.role].0;
        self.household_name.update(cx, |i, cx| i.set_text("", cx));
        self.setup_call(
            cx,
            move |v| v.add_household_member(&name, role).map(|_| ()),
            2,
        );
    }
    fn remove_household(&mut self, entity: String, cx: &mut Context<Self>) {
        self.setup_call(cx, move |v| v.remove_household_member(&entity), 2);
    }

    /// Folder picker for a dump; also used by Imports.
    pub(super) fn choose_dump_folders(&mut self, cx: &mut Context<Self>) {
        let generation = self.generation;
        let task = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: true,
            prompt: Some("Give to ME.".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = task.await {
                let _ = this.update(cx, |this, cx| {
                    if this.generation == generation {
                        this.plan_dump(paths, cx);
                    }
                });
            }
        })
        .detach();
    }
    /// Metadata-only walk; nothing is read or stored until Start.
    pub(super) fn plan_dump(&mut self, roots: Vec<PathBuf>, cx: &mut Context<Self>) {
        if self.graph.planning {
            return;
        }
        self.graph.planning = true;
        let generation = self.generation;
        let walk = roots.clone();
        let task = cx
            .background_executor()
            .spawn(async move { me_core::plan_folder_import(&walk) });
        cx.spawn(async move |this, cx| {
            let plan = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.graph.planning = false;
                this.graph.plan = Some((roots, plan));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn start_dump(&mut self, cx: &mut Context<Self>) {
        let Some((_, plan)) = self.graph.plan.take() else {
            return;
        };
        if self.graph.dump_cancel.is_some() || plan.files.is_empty() {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.graph.dump_cancel = Some(cancel.clone());
        (
            self.graph.dump_saved,
            self.graph.dump_duplicates,
            self.graph.dump_failed,
        ) = (0, 0, 0);
        self.page = Page::Imports;
        let setup = self.identity_setup_visible();
        let session = self.session.clone();
        let generation = self.generation;
        let (tx, rx) = std::sync::mpsc::channel();
        let files = plan.files;
        cx.background_executor()
            .spawn(async move {
                let batch = session
                    .lock()
                    .ok()
                    .and_then(|mut g| g.as_mut().map(|v| v.begin_import_batch("Folder import")))
                    .transpose()
                    .ok()
                    .flatten();
                let _ = tx.send(Err(batch.clone()));
                for path in files {
                    if cancel.load(Ordering::SeqCst) {
                        break;
                    }
                    // Short serialized writes; reading and analysis happen elsewhere.
                    let outcome = session
                        .lock()
                        .map_err(|_| "Vault unavailable.".to_string())
                        .and_then(|mut g| {
                            g.as_mut()
                                .ok_or("Vault locked.".to_string())?
                                .import_envelope_file(&path, SourceKind::Folder, batch.as_deref())
                                .map_err(|e| e.to_string())
                        });
                    if tx.send(Ok(outcome.map(|o| o.duplicate))).is_err() {
                        break;
                    }
                }
            })
            .detach();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let mut finished = false;
            while !finished {
                let mut changed = false;
                loop {
                    match rx.try_recv() {
                        Ok(message) => {
                            changed = true;
                            let _ = this.update(cx, |this, _| match message {
                                Err(Some(batch)) => {
                                    this.graph.batch = Some(BatchProgress {
                                        id: batch,
                                        ..Default::default()
                                    })
                                }
                                Err(None) => {}
                                Ok(Ok(true)) => this.graph.dump_duplicates += 1,
                                Ok(Ok(false)) => this.graph.dump_saved += 1,
                                Ok(Err(_)) => this.graph.dump_failed += 1,
                            });
                        }
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            finished = true;
                            break;
                        }
                        Err(_) => break,
                    }
                }
                if changed {
                    let _ = this.update(cx, |this, cx| {
                        if this.generation == generation {
                            this.refresh_imports(cx);
                            this.kick_auto_queue(cx);
                            this.refresh_graph(cx);
                        }
                    });
                }
                if !finished {
                    executor.timer(std::time::Duration::from_millis(400)).await;
                }
            }
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.graph.dump_cancel = None;
                this.refresh_imports(cx);
                this.refresh_search(cx);
                this.kick_auto_queue(cx);
                this.refresh_graph(cx);
                cx.notify();
            });
        })
        .detach();
        if setup {
            self.finish_identity_setup(cx);
        }
        cx.notify();
    }
    pub(super) fn finish_identity_setup(&mut self, cx: &mut Context<Self>) {
        let session = self.session.clone();
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            let mut guard = session.lock().map_err(|_| me_core::Error::Format)?;
            guard
                .as_mut()
                .ok_or(me_core::Error::Format)?
                .complete_identity_setup()
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                if result.is_ok()
                    && let Some(a) = this.graph.anchors.as_mut()
                {
                    a.setup_complete = true;
                }
                this.focus_filter_on_ready = true;
                this.refresh_graph(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn decide(
        &mut self,
        key: String,
        cx: &mut Context<Self>,
        work: impl FnOnce(&mut Vault) -> me_core::Result<()> + Send + 'static,
    ) {
        if !self.graph.busy.insert(key.clone()) {
            return;
        }
        let session = self.session.clone();
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            let mut guard = session.lock().map_err(|_| me_core::Error::Format)?;
            work(guard.as_mut().ok_or(me_core::Error::Format)?)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.graph.busy.remove(&key);
                if let Err(e) = result {
                    this.notice = Some(e.to_string());
                }
                this.refresh_graph(cx);
                this.refresh_search(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Reduce side of the map-reduce: spelling-only conflicts and organization
    /// duplicates, judged in batched TypeSafe requests once the queue is idle.
    pub(super) fn run_reduce(&mut self, cx: &mut Context<Self>) {
        let count = self.graph.constellation.nodes.len() + self.graph.checks.len();
        if self.graph.reduce_running
            || self.graph.reduce_done_for == count
            || !self.active_imports.is_empty()
            || self.import_pause.is_some()
        {
            return;
        }
        self.graph.reduce_running = true;
        self.graph.reduce_done_for = count;
        let session = self.session.clone();
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            let cancel = AtomicBool::new(false);
            let (conflicts, merges) = {
                let guard = session
                    .lock()
                    .map_err(|_| "Vault unavailable.".to_string())?;
                let v = guard.as_ref().ok_or("Vault locked.".to_string())?;
                (
                    v.open_value_conflicts(40).map_err(|e| e.to_string())?,
                    v.merge_candidates(40).map_err(|e| e.to_string())?,
                )
            };
            if conflicts.is_empty() && merges.is_empty() {
                return Ok::<_, String>(());
            }
            let mut decisions =
                me_agent::typesafe::TypeSafe::configured().map_err(|f| f.message)?;
            let (same, _) =
                me_agent::graph_pipeline::judge_equivalence(&conflicts, &mut decisions, &cancel)
                    .map_err(|f| f.message)?;
            let (aligned, _) =
                me_agent::graph_pipeline::align_organizations(&merges, &mut decisions, &cancel)
                    .map_err(|f| f.message)?;
            let mut guard = session
                .lock()
                .map_err(|_| "Vault unavailable.".to_string())?;
            let v = guard.as_mut().ok_or("Vault locked.".to_string())?;
            for (keep, duplicate) in same {
                let _ = v.resolve_equivalent_values(&keep, &duplicate);
            }
            for j in aligned {
                v.record_merge_judgment(&j.a, &j.b, j.score, j.propose)
                    .map_err(|e| e.to_string())?;
            }
            Ok(())
        });
        cx.spawn(async move |this, cx| {
            let _ = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.graph.reduce_running = false;
                this.refresh_graph(cx);
            });
        })
        .detach();
    }

    /// On demand: map an existence question over lazy documents, then read the
    /// few that answer it in depth (the grounded extraction pipeline).
    fn start_deep_search(&mut self, cx: &mut Context<Self>) {
        let query = self.filter.query.clone();
        if query.is_empty() || self.graph.deep_running || !self.ai_ready() {
            return;
        }
        self.graph.deep_running = true;
        self.graph.deep_query = Some(query.clone());
        self.graph.deep_found.clear();
        self.graph.deep_error = None;
        let session = self.session.clone();
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            let documents = {
                let guard = session
                    .lock()
                    .map_err(|_| "Vault unavailable.".to_string())?;
                let v = guard.as_ref().ok_or("Vault locked.".to_string())?;
                v.lazy_document_excerpts(400).map_err(|e| e.to_string())?
            };
            let mut decisions =
                me_agent::typesafe::TypeSafe::configured().map_err(|f| f.message)?;
            let (found, _) = me_agent::graph_pipeline::search_documents(
                &query,
                &documents,
                &mut decisions,
                &AtomicBool::new(false),
            )
            .map_err(|f| f.message)?;
            let guard = session
                .lock()
                .map_err(|_| "Vault unavailable.".to_string())?;
            let v = guard.as_ref().ok_or("Vault locked.".to_string())?;
            let mut items = Vec::new();
            for source in found.iter().take(3) {
                if let Some(item) = v.source_item(source).map_err(|e| e.to_string())? {
                    let title = documents
                        .iter()
                        .find(|d| &d.0 == source)
                        .map_or_else(String::new, |d| d.1.clone());
                    items.push((item, title));
                }
            }
            Ok::<_, String>(items)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.graph.deep_running = false;
                match result {
                    Ok(items) => {
                        for (item, _) in &items {
                            this.run_document(*item, true, cx);
                        }
                        this.graph.deep_found = items;
                    }
                    Err(e) => this.graph.deep_error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn deep_search_panel(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let current = self.graph.deep_query.as_deref() == Some(self.filter.query.as_str());
        if self.filter.query.is_empty() || self.graph.lazy_documents == 0 || !self.ai_ready() {
            return div().into_any_element();
        }
        div().mt(px(space::LG)).p(px(space::LG)).rounded(px(radius::STANDARD)).border_1().border_color(rgb(LINE)).bg(rgb(SURFACE))
            .flex().flex_col().gap(px(space::SM))
            .child(div().flex().items_center().gap(px(space::SM))
                .child(icon(Icon::Spark, IconSize::Medium, ACCENT))
                .child(div().type_style(Type::Label).child(format!(
                    "{} filed {} not read in detail yet",
                    self.graph.lazy_documents,
                    if self.graph.lazy_documents == 1 { "document is" } else { "documents are" }
                ))))
            .child(div().type_style(Type::Small).text_color(rgb(MUTED)).child(match (current, self.graph.deep_running, self.graph.deep_found.len()) {
                (true, true, _) => "Checking which ones answer your search…".to_owned(),
                (true, false, 0) if self.graph.deep_error.is_none() => "None of them seems to answer this search.".to_owned(),
                (true, false, n) if n > 0 => format!(
                    "Reading {} in detail: {}. New details appear in Review.",
                    if n == 1 { "1 document".to_owned() } else { format!("{n} documents") },
                    self.graph.deep_found.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join(", ")
                ),
                _ => "ME. can check them for this search and read the matching ones in detail.".to_owned(),
            }))
            .when_some(self.graph.deep_error.clone().filter(|_| current), |s, e| s.child(div().type_style(Type::Small).text_color(rgb(DANGER)).child(e)))
            .when(!current || (!self.graph.deep_running && self.graph.deep_error.is_some()), |s| s.child(
                secondary_action().id("deep-search").hover(|s| s.bg(rgb(HOVER)))
                    .on_click(cx.listener(|this, _, _, cx| this.start_deep_search(cx)))
                    .child(icon(Icon::Search, IconSize::Medium, INK)).child("Look inside filed documents")))
            .into_any_element()
    }
    fn setup_shell(
        &self,
        title: String,
        subtitle: &'static str,
        body: gpui::AnyElement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let step = self.graph.setup_step;
        let wide = window.viewport_size().width >= px(layout::ONBOARDING_WIDTH);
        let rail = step_rail(SETUP_STEPS, step);
        let panel = div()
            .w(px(layout::ONBOARDING_PANEL_WIDTH))
            .max_w_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap(px(space::XL))
            .child(rail)
            .child(
                modal_panel(layout::ONBOARDING_PANEL_WIDTH, self.motion_enabled())
                    .gap(px(space::LG))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(space::SM))
                            .child(heading(title))
                            .child(
                                div()
                                    .type_style(Type::Body)
                                    .text_color(rgb(MUTED))
                                    .child(subtitle),
                            ),
                    )
                    .when_some(self.graph.setup_error.clone(), |s, e| {
                        s.child(
                            div()
                                .p(px(space::MD))
                                .rounded(px(radius::STANDARD))
                                .bg(rgb(DANGER_SURFACE))
                                .text_color(rgb(DANGER))
                                .type_style(Type::Small)
                                .child(e),
                        )
                    })
                    .child(body),
            );
        // The fingerprint gains colour with every answer: this becomes ME.
        let tint =
            (step as f32 * sky::ANSWER_TINT).min(crate::design_system::motion::FINGERPRINT_OPACITY);
        let art = div()
            .relative()
            .size(px(layout::FINGERPRINT_ART_SIZE))
            .child(onboarding_fingerprint(1.))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .child(icon(Icon::Fingerprint, IconSize::Identity, ACCENT).opacity(tint)),
            );
        div()
            .id(("identity-setup", step))
            .size_full()
            .overflow_y_scroll()
            .px(px(space::XXL))
            .flex()
            .flex_col()
            .key_context("Me")
            .track_focus(&self.focus)
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.plan_dump(paths.paths().to_vec(), cx)
            }))
            .drag_over::<ExternalPaths>(|s, _, _, _| s.border_2().border_color(rgb(ACCENT)))
            .child(
                div()
                    .w(px(layout::ONBOARDING_WIDTH))
                    .max_w_full()
                    .mx_auto()
                    .my_auto()
                    .pt(px(space::TITLEBAR))
                    .pb(px(space::PAGE_TOP))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(space::PAGE))
                    .when(wide, |s| {
                        s.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap(px(space::SECTION))
                                .child(brand_signature(false, window))
                                .child(art)
                                .child(
                                    div()
                                        .type_style(Type::Caption)
                                        .text_color(rgb(MUTED))
                                        .flex()
                                        .items_center()
                                        .gap(px(space::SM))
                                        .child(icon(Icon::Fingerprint, IconSize::Medium, ACCENT))
                                        .child("Everything stays encrypted on your device."),
                                ),
                        )
                    })
                    .child(
                        div()
                            .w(px(layout::ONBOARDING_PANEL_WIDTH))
                            .max_w_full()
                            .flex()
                            .flex_col()
                            .gap(px(space::XL))
                            .when(!wide, |s| s.child(brand_signature(true, window)))
                            .child(motion::page(
                                panel.into_any_element(),
                                step,
                                self.motion_enabled(),
                            )),
                    ),
            )
            .into_any_element()
    }

    /// Conversational, one question per screen. It never blocks the import.
    pub(super) fn identity_setup_view(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let busy = self.graph.setup_busy;
        let continue_button = |id: &'static str, label: &'static str| {
            primary_action()
                .id(id)
                .h(px(layout::CONTROL_LARGE))
                .hover(|s| s.bg(rgb(PRIMARY_HOVER)))
                .when(busy, |s| s.cursor_default())
                .child(label)
                .child(action_indicator(busy, self.motion_enabled()))
        };
        let skip = |id: &'static str, label: &'static str, next: usize, cx: &mut Context<Self>| {
            div()
                .id(id)
                .type_style(Type::Small)
                .text_color(rgb(MUTED))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.graph.setup_error = None;
                    this.graph.setup_step = next;
                    cx.notify();
                }))
                .child(label)
        };
        let anchors = self
            .graph
            .anchors
            .clone()
            .unwrap_or_else(|| IdentityAnchors {
                self_entity: String::new(),
                self_name: String::new(),
                aliases: vec![],
                birth_date: None,
                household: vec![],
                setup_complete: false,
            });
        match self.graph.setup_step {
            0 => self.setup_shell(
                "Hi. Who are you?".into(),
                "This becomes ME. Your name lets ME. recognize you in every document you add.",
                div().flex().flex_col().gap(px(space::LG))
                    .child(self.editor_field("Your full name", self.identity_name.clone(), window, cx))
                    .child(self.editor_field("Ever gone by another name? (optional, comma-separated)", self.identity_aliases.clone(), window, cx))
                    .child(continue_button("identity-name", "That’s me").on_click(cx.listener(|this, _, _, cx| this.save_identity_name(cx))))
                    .into_any_element(),
                window,
                cx,
            ),
            1 => self.setup_shell(
                format!("Nice, {}. When were you born?", first_name(&anchors.self_name)),
                "Your birthday tells your documents apart from those of people with the same name.",
                div().flex().flex_col().gap(px(space::LG))
                    .child(self.editor_field("Date of birth (e.g. 14.03.1988)", self.identity_birth.clone(), window, cx))
                    .child(continue_button("identity-birth", "Continue").on_click(cx.listener(|this, _, _, cx| this.save_birth_date(cx))))
                    .child(skip("identity-birth-skip", "Skip for now", 2, cx))
                    .into_any_element(),
                window,
                cx,
            ),
            2 => {
                let members = div().flex().flex_wrap().gap(px(space::SM)).children(anchors.household.iter().map(|m| {
                    let entity = m.entity.clone();
                    div().id(SharedString::from(format!("member-{}", m.entity))).h(px(layout::CONTROL_COMPACT)).px(px(space::MD))
                        .rounded(px(radius::STANDARD)).border_1().border_color(rgb(LINE)).bg(rgb(SURFACE))
                        .flex().items_center().gap(px(space::SM)).type_style(Type::Small)
                        .child(icon(Icon::User, IconSize::Small, ACCENT))
                        .child(format!("{} · {}", m.name, ROLES.iter().find(|r| r.0 == m.role).map_or("Other", |r| r.1)))
                        .child(div().id(SharedString::from(format!("remove-{}", m.entity))).cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| this.remove_household(entity.clone(), cx)))
                            .child(icon(Icon::Close, IconSize::Small, MUTED)))
                }));
                let roles = div().flex().gap(px(space::SM)).children(ROLES.iter().enumerate().map(|(index, (_, label))| {
                    let selected = index == self.graph.role;
                    div().id(("household-role", index)).h(px(layout::CONTROL_COMPACT)).px(px(space::MD))
                        .rounded(px(radius::STANDARD)).border_1().border_color(rgb(if selected { ACCENT } else { LINE }))
                        .bg(rgb(if selected { HOVER } else { SURFACE })).flex().items_center().cursor_pointer()
                        .type_style(Type::Small).text_color(rgb(if selected { ACCENT } else { INK }))
                        .on_click(cx.listener(move |this, _, _, cx| { this.graph.role = index; cx.notify(); }))
                        .child(*label)
                }));
                self.setup_shell(
                    "Who lives with you?".into(),
                    "Partner, children or parents get their own place in ME. Everyone else stays a name on a letter.",
                    div().flex().flex_col().gap(px(space::LG))
                        .when(!anchors.household.is_empty(), |s| s.child(members))
                        .child(self.editor_field("Name", self.household_name.clone(), window, cx))
                        .child(roles)
                        .child(secondary_action().id("household-add").hover(|s| s.bg(rgb(HOVER)))
                            .on_click(cx.listener(|this, _, _, cx| this.add_household(cx)))
                            .child(icon(Icon::Plus, IconSize::Medium, INK)).child("Add to household"))
                        .child(continue_button("identity-household", "Continue").on_click(cx.listener(|this, _, _, cx| {
                            this.graph.setup_error = None;
                            this.graph.setup_step = 3;
                            cx.notify();
                        })))
                        .into_any_element(),
                    window,
                    cx,
                )
            }
            _ => self.setup_shell(
                format!("Nice to meet you, {}. Now give ME. everything.", first_name(&anchors.self_name)),
                "Contracts, passports, tax mail, payslips: whole folders are fine. Duplicates and junk are sorted out for you.",
                div().flex().flex_col().gap(px(space::LG))
                    .child(div().id("dump-drop").p(px(space::XXL)).rounded(px(radius::STANDARD)).border_1().border_color(rgb(DECORATIVE))
                        .bg(rgb(BG)).flex().flex_col().items_center().gap(px(space::SM)).cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| this.choose_dump_folders(cx)))
                        .child(icon(Icon::Folder, IconSize::Large, ACCENT))
                        .child(div().type_style(Type::Label).child(if self.graph.planning { "Looking through your folders…" } else { "Drop folders here" }))
                        .child(div().type_style(Type::Caption).text_color(rgb(MUTED)).child("PDF · JPG · HEIC · PNG · DOCX · EML")))
                    .child(continue_button("dump-choose", "Choose folders…").on_click(cx.listener(|this, _, _, cx| this.choose_dump_folders(cx))))
                    .child(div().type_style(Type::Caption).text_color(rgb(MUTED))
                        .child("Files are read on this device first. Only recognized text goes to TypeSafe to sort documents and pick values; OpenAI is asked only when something important is still missing."))
                    .child(div().id("dump-skip").type_style(Type::Small).text_color(rgb(MUTED)).cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| this.finish_identity_setup(cx))).child("Skip for now"))
                    .into_any_element(),
                window,
                cx,
            ),
        }
    }

    /// Summary before a dump starts. Nothing has been read or stored yet.
    pub(super) fn dump_summary_modal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (roots, plan) = self.graph.plan.as_ref().expect("plan");
        let folders = roots.len();
        let row = |label: String, value: String| {
            div()
                .flex()
                .justify_between()
                .gap(px(space::LG))
                .type_style(Type::Body)
                .child(div().text_color(rgb(MUTED)).child(label))
                .child(div().font_family(font::MONO).child(value))
        };
        self.overlay(cx).child(modal_panel(486., self.motion_enabled())
            .child(heading("Ready when you are"))
            .child(div().type_style(Type::Body).text_color(rgb(MUTED)).child(format!(
                "ME. found {} in {} {}.",
                if plan.files.len() == 1 { "1 file".to_owned() } else { format!("{} files", plan.files.len()) },
                folders,
                if folders == 1 { "place" } else { "places" }
            )))
            .child(div().flex().flex_col().gap(px(space::SM))
                .child(row("Documents to add".into(), plan.files.len().to_string()))
                .child(row("Size".into(), mebibytes(plan.bytes)))
                .child(row("Formats ME. can’t read".into(), plan.unsupported.to_string()))
                .child(row("Empty, too large or linked".into(), plan.skipped.to_string()))
                .child(row("Hidden system files".into(), plan.hidden.to_string())))
            .when(plan.truncated, |s| s.child(div().p(px(space::MD)).rounded(px(radius::STANDARD)).bg(rgb(WARNING_SURFACE))
                .text_color(rgb(WARNING)).type_style(Type::Small)
                .child(format!("Only the first {} files are added now. Add the rest afterwards.", me_core::MAX_PLANNED_FILES))))
            .child(div().type_style(Type::Caption).text_color(rgb(MUTED)).child(
                "Duplicates are recognized by their content and stored once. About one TypeSafe decision per document, plus one for identity, tax, insurance, payslip, bank, vehicle, housing and contract documents. OpenAI calls are limited per import and only fill missing important values."))
            .child(div().flex().justify_end().gap(px(space::SM))
                .child(secondary_action().id("dump-cancel").hover(|s| s.bg(rgb(HOVER)))
                    .on_click(cx.listener(|this, _, _, cx| { this.graph.plan = None; cx.notify(); }))
                    .child("Not now"))
                .child(primary_action().id("dump-start").hover(|s| s.bg(rgb(PRIMARY_HOVER)))
                    .when(plan.files.is_empty(), |s| s.cursor_default())
                    .on_click(cx.listener(|this, _, _, cx| this.start_dump(cx)))
                    .child("Start").child(icon(Icon::Arrow, IconSize::Medium, SURFACE)))))
    }

    /// Nodes are entities; the most connected ones orbit the user. Families that
    /// stay collapsed appear as clusters below, with a plain-text ticker.
    pub(super) fn constellation_panel(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let map = &self.graph.constellation;
        let dumping = self.graph.dump_cancel.is_some();
        let queued = self
            .import_jobs
            .iter()
            .filter(|j| j.state == "queued" && j.processable)
            .count();
        let active = self.active_imports.len();
        let batch = self.graph.batch.as_ref();
        let (done, total) = batch.map_or((0, 0), |b| (b.finished, b.files));
        let fraction = (total > 0).then(|| done as f32 / total as f32);
        let working = dumping || active > 0 || queued > 0;
        // Honeycomb spiral around the center cell, in the Knowledge map's cell language.
        let positions = spiral(map.nodes.len());
        let labels: Vec<(f32, f32, String, bool, bool)> = map
            .nodes
            .iter()
            .zip(&positions)
            .map(|(n, (x, y))| (*x, *y, n.label.clone(), n.check, n.anchor))
            .collect();
        let index: BTreeMap<&str, (f32, f32)> = map
            .nodes
            .iter()
            .map(|n| n.id.as_str())
            .zip(positions.iter().copied())
            .collect();
        let edges: Vec<((f32, f32), (f32, f32))> = map
            .edges
            .iter()
            .filter_map(|(a, b)| Some((*index.get(a.as_str())?, *index.get(b.as_str())?)))
            .collect();
        let cells = labels.clone();
        let sky = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let center = point(
                    f32::from(bounds.origin.x) + f32::from(bounds.size.width) / 2.,
                    f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2.,
                );
                let at = |(x, y): (f32, f32)| point(center.x + x, center.y + y);
                let mut lines = PathBuilder::stroke(px(crate::design_system::knowledge::BORDER));
                for (a, b) in &edges {
                    let (a, b) = (at(*a), at(*b));
                    lines.move_to(point(px(a.x), px(a.y)));
                    lines.line_to(point(px(b.x), px(b.y)));
                }
                if let Ok(path) = lines.build() {
                    let mut c = rgb(DECORATIVE);
                    c.a = sky::EDGE_OPACITY;
                    window.paint_path(path, c);
                }
                for (x, y, _, check, anchor) in &cells {
                    let p = at((*x, *y));
                    let outline = knowledge_canvas::hex_outline(p, sky::CELL_RADIUS);
                    let mut fill = PathBuilder::fill();
                    knowledge_canvas::rounded_path(&mut fill, &outline, radius::STANDARD);
                    if let Ok(path) = fill.build() {
                        window.paint_path(path, rgb(SURFACE));
                    }
                    let mut border =
                        PathBuilder::stroke(px(crate::design_system::knowledge::BORDER));
                    knowledge_canvas::rounded_path(&mut border, &outline, radius::STANDARD);
                    if let Ok(path) = border.build() {
                        window.paint_path(
                            path,
                            rgb(if *check {
                                WARNING
                            } else if *anchor {
                                ACCENT
                            } else {
                                DECORATIVE
                            }),
                        );
                    }
                }
            },
        )
        .size_full();
        div().flex().flex_col().gap(px(space::MD))
            .child(div().flex().items_center().justify_between().gap(px(space::LG))
                .child(div().type_style(Type::Section).child("Your world is growing"))
                .child(div().type_style(Type::Caption).font_family(font::MONO).text_color(rgb(MUTED)).child(match fraction {
                    Some(f) => format!("{done} / {total} files · {:.0}%", f * 100.),
                    None => format!("{} places known", map.nodes.len()),
                })))
            .when(total > 0 || working, |s| s.child(progress_bar(fraction, ACCENT, self.active_imports.values().next().map_or(0., |a| a.started.elapsed().as_secs_f32() / 3.))))
            .child(
                div().relative().w_full().h(px(sky::HEIGHT)).rounded(px(radius::STANDARD)).border_1().border_color(rgb(LINE)).bg(rgb(BG)).overflow_hidden()
                    .child(div().absolute().inset_0().child(sky))
                    // A zero-size anchor at the exact center; cells are placed around it.
                    .child(div().absolute().inset_0().flex().items_center().justify_center().child(
                        div().relative().size_0().children(labels.into_iter().map(|(x, y, label, check, anchor)| {
                            div().absolute().left(px(x - sky::LABEL_WIDTH / 2.)).top(px(y - sky::LABEL_HEIGHT / 2.))
                                .w(px(sky::LABEL_WIDTH)).h(px(sky::LABEL_HEIGHT)).flex().items_center().justify_center().overflow_hidden()
                                .child({
                                    let (first, second) = two_lines(&label);
                                    div().w_full().flex().flex_col().items_center().type_style(Type::Caption)
                                        .text_color(rgb(if check { WARNING } else if anchor { ACCENT } else { INK }))
                                        .when(anchor, |s| s.font_weight(font::EMPHASIS))
                                        .child(div().max_w_full().truncate().child(first))
                                        .when_some(second, |s, second| s.child(div().max_w_full().truncate().child(second)))
                                })
                        })),
                    )),
            )
            .child(div().type_style(Type::Small).text_color(rgb(MUTED)).child(match (&map.latest, dumping) {
                (Some(latest), _) => format!("Latest: {latest}"),
                (None, true) => "Saving your originals…".into(),
                (None, false) => "Add documents and ME. starts connecting the dots.".into(),
            }))
            .when(!map.clusters.is_empty(), |s| s.child(div().flex().flex_wrap().gap(px(space::SM)).children(map.clusters.iter().map(|(label, n)| {
                div().h(px(layout::CONTROL_COMPACT)).px(px(space::MD)).rounded(px(radius::STANDARD)).border_1().border_color(rgb(LINE))
                    .bg(rgb(SURFACE)).flex().items_center().gap(px(space::SM)).type_style(Type::Small)
                    .child(label.clone()).child(div().font_family(font::MONO).text_color(rgb(MUTED)).child(n.to_string()))
            }))))
            .when(self.graph.dump_cancel.is_some() || self.graph.dump_saved + self.graph.dump_duplicates + self.graph.dump_failed > 0, |s| s.child(
                div().flex().items_center().gap(px(space::LG)).type_style(Type::Caption).text_color(rgb(MUTED))
                    .child(format!("{} saved", self.graph.dump_saved))
                    .child(format!("{} duplicates skipped", self.graph.dump_duplicates))
                    .when(self.graph.dump_failed > 0, |s| s.child(div().text_color(rgb(DANGER)).child(format!("{} not imported", self.graph.dump_failed))))
                    .when(dumping, |s| s.child(div().id("dump-stop").text_color(rgb(ACCENT)).cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(c) = &this.graph.dump_cancel { c.store(true, Ordering::SeqCst); }
                            cx.notify();
                        })).child("Stop adding")))))
            .when_some(batch.filter(|b| b.openai_calls >= b.openai_allowance && b.openai_allowance > 0), |s, b| {
                let id = b.id.clone();
                s.child(div().p(px(space::MD)).rounded(px(radius::STANDARD)).bg(rgb(WARNING_SURFACE)).flex().flex_col().gap(px(space::XS))
                    .child(div().type_style(Type::Small).text_color(rgb(WARNING)).child("OpenAI allowance for this import is used up. Documents continue with local reading and TypeSafe only."))
                    .child(div().id("batch-extend").type_style(Type::Small).text_color(rgb(ACCENT)).cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let id = id.clone();
                            this.decide(format!("extend-{id}"), cx, move |v| v.extend_batch_allowance(&id));
                        })).child("Allow more OpenAI calls for this import")))
            })
            .into_any_element()
    }

    /// Automatic values worth a look, household and merge proposals.
    pub(super) fn quick_checks_panel(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let graph = &self.graph;
        if graph.checks.is_empty() && graph.households.is_empty() && graph.merges.is_empty() {
            return div().into_any_element();
        }
        let action = |id: String, label: &'static str, primary: bool| {
            let base = if primary {
                primary_action()
            } else {
                secondary_action()
            };
            base.id(SharedString::from(id))
                .h(px(layout::CONTROL_COMPACT))
                .hover(move |s| s.bg(rgb(if primary { PRIMARY_HOVER } else { HOVER })))
                .child(label)
        };
        let card = || {
            div()
                .p(px(space::LG))
                .bg(rgb(SURFACE))
                .rounded(px(radius::STANDARD))
                .border_1()
                .border_color(rgb(LINE))
                .flex()
                .flex_col()
                .gap(px(space::MD))
        };
        div()
            .flex()
            .flex_col()
            .gap(px(space::MD))
            .mb(px(space::SECTION))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::XS))
                    .child(div().type_style(Type::Section).child("Quick checks"))
                    .child(div().type_style(Type::Body).text_color(rgb(MUTED)).child(
                        "ME. added these automatically. A short look keeps your details right.",
                    )),
            )
            .children(graph.households.iter().map(|p| {
                let (add, dismiss) = (p.name.clone(), p.name.clone());
                card()
                    .child(
                        div()
                            .type_style(Type::Label)
                            .child(format!("Add {} to your household?", p.name)),
                    )
                    .child(
                        div()
                            .type_style(Type::Small)
                            .text_color(rgb(MUTED))
                            .child(format!(
                                "{} documents are about {}.",
                                p.documents,
                                first_name(&p.name)
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(space::SM))
                            .child(
                                action(format!("household-add-{}", p.name), "Add", true).on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        let name = add.clone();
                                        this.decide(format!("household-{name}"), cx, move |v| {
                                            v.add_household_member(&name, "other").map(|_| ())
                                        });
                                    }),
                                ),
                            )
                            .child(
                                action(format!("household-dismiss-{}", p.name), "Not now", false)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let name = dismiss.clone();
                                        this.decide(format!("household-{name}"), cx, move |v| {
                                            v.dismiss_household_proposal(&name)
                                        });
                                    })),
                            ),
                    )
            }))
            .children(graph.merges.iter().map(|m| {
                let (a, b) = (m.a.clone(), m.b.clone());
                let (a2, b2) = (m.a.clone(), m.b.clone());
                card()
                    .child(div().type_style(Type::Label).child("Same organization?"))
                    .child(
                        div()
                            .type_style(Type::Body)
                            .font_family(font::MONO)
                            .child(format!("{}  ·  {}", m.a_label, m.b_label)),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(space::SM))
                            .child(
                                action(format!("merge-{}-{}", m.a, m.b), "Same", true).on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        let (a, b) = (a.clone(), b.clone());
                                        this.decide(format!("merge-{a}"), cx, move |v| {
                                            v.decide_merge_proposal(&a, &b, true)
                                        });
                                    }),
                                ),
                            )
                            .child(
                                action(format!("keep-{}-{}", m.a, m.b), "Different", false)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let (a, b) = (a2.clone(), b2.clone());
                                        this.decide(format!("merge-{a}"), cx, move |v| {
                                            v.decide_merge_proposal(&a, &b, false)
                                        });
                                    })),
                            ),
                    )
            }))
            .children(graph.checks.iter().map(|group| {
                card()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(space::SM))
                            .child(icon(
                                match group.entity_kind.as_str() {
                                    "person" => Icon::User,
                                    "government_id" => Icon::Card,
                                    "bank_account" => Icon::Card,
                                    "address" => Icon::Location,
                                    "organization" => Icon::Grid,
                                    _ => Icon::Document,
                                },
                                IconSize::Medium,
                                ACCENT,
                            ))
                            .child(div().type_style(Type::Label).child(format!(
                                "{}: {} worth a look",
                                group.entity_label,
                                if group.items.len() == 1 {
                                    "1 thing".to_owned()
                                } else {
                                    format!("{} things", group.items.len())
                                }
                            ))),
                    )
                    .children(group.items.iter().map(|item| {
                        let (yes, no) = (item.assertion.clone(), item.assertion.clone());
                        let busy = self.graph.busy.contains(&item.assertion);
                        div()
                            .pt(px(space::MD))
                            .border_t_1()
                            .border_color(rgb(LINE))
                            .flex()
                            .flex_col()
                            .gap(px(space::SM))
                            .child(
                                div()
                                    .flex()
                                    .justify_between()
                                    .gap(px(space::LG))
                                    .child(
                                        div()
                                            .type_style(Type::Small)
                                            .text_color(rgb(MUTED))
                                            .child(item.label.clone()),
                                    )
                                    .child(
                                        div()
                                            .type_style(Type::Caption)
                                            .text_color(rgb(WARNING))
                                            .child(match item.reason.as_str() {
                                                "conflict" => "Differs from another document",
                                                _ => "Worth a quick look",
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .type_style(Type::Value)
                                    .font_family(font::MONO)
                                    .child(item.value.clone()),
                            )
                            .when_some(item.source_title.clone(), |s, t| {
                                s.child(
                                    div()
                                        .type_style(Type::Caption)
                                        .text_color(rgb(MUTED))
                                        .child(format!("From {t}")),
                                )
                            })
                            .child(
                                div()
                                    .flex()
                                    .gap(px(space::SM))
                                    .child(
                                        action(
                                            format!("check-yes-{}", item.assertion),
                                            if busy { "Saving…" } else { "Looks right" },
                                            true,
                                        )
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                let id = yes.clone();
                                                this.decide(id.clone(), cx, move |v| {
                                                    v.confirm_assertion(&id)
                                                });
                                            }),
                                        ),
                                    )
                                    .child(
                                        action(
                                            format!("check-no-{}", item.assertion),
                                            "Not right",
                                            false,
                                        )
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                let id = no.clone();
                                                this.decide(id.clone(), cx, move |v| {
                                                    v.reject_assertion(&id)
                                                });
                                            }),
                                        ),
                                    ),
                            )
                    }))
            }))
            .into_any_element()
    }
}

/// Splits a cell label into at most two lines at word boundaries; each line truncates.
fn two_lines(label: &str) -> (String, Option<String>) {
    const LINE: usize = 11;
    let mut first = String::new();
    let mut words = label.split_whitespace().peekable();
    while let Some(word) = words.peek() {
        if !first.is_empty() && first.chars().count() + 1 + word.chars().count() > LINE {
            break;
        }
        if !first.is_empty() {
            first.push(' ');
        }
        first.push_str(word);
        words.next();
    }
    let rest = words.collect::<Vec<_>>().join(" ");
    (first, (!rest.is_empty()).then_some(rest))
}

/// Axial honeycomb spiral: the center cell, then ring after ring, in pixels.
fn spiral(count: usize) -> Vec<(f32, f32)> {
    const DIRECTIONS: [(i32, i32); 6] = [(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)];
    let mut cells = vec![(0i32, 0i32)];
    let mut ring = 1;
    while cells.len() < count {
        let (mut q, mut r) = (-ring, ring);
        for (dq, dr) in DIRECTIONS {
            for _ in 0..ring {
                cells.push((q, r));
                q += dq;
                r += dr;
            }
        }
        ring += 1;
    }
    cells.truncate(count);
    let size = sky::CELL_RADIUS + space::XS;
    cells
        .into_iter()
        .map(|(q, r)| {
            (
                3f32.sqrt() * size * (q as f32 + r as f32 / 2.),
                1.5 * size * r as f32,
            )
        })
        .collect()
}
