use super::*;
use gpui::{ElementId, SharedString};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Page {
    Search,
    Review,
    Browser,
    Imports,
}
impl MeApp {
    pub(super) fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(layout::SIDEBAR_WIDTH))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(SIDEBAR))
            .border_r_1()
            .border_color(rgb(LINE))
            .px(px(space::LG))
            .pt(px(space::TITLEBAR))
            .pb(px(space::XXL))
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(space::MD))
                    .flex()
                    .items_center()
                    .gap(px(space::SM))
                    .child(icon(Icon::Fingerprint, IconSize::Brand, INK))
                    .child(
                        div()
                            .type_style(Type::BrandSmall)
                            .font_weight(font::EMPHASIS)
                            .child("ME."),
                    ),
            )
            .child(
                div()
                    .px(px(space::MD))
                    .mt(px(space::SECTION_LOOSE))
                    .mb(px(space::MD))
                    .child(eyebrow("WORKSPACE")),
            )
            .children(
                [
                    (Page::Search, "Search", Icon::Search),
                    (Page::Review, "Review", Icon::Check),
                    (Page::Browser, "Browser", Icon::Folder),
                    (Page::Imports, "Imports", Icon::Upload),
                ]
                .into_iter()
                .map(|(page, label, symbol)| {
                    let active = self.page == page && !self.show_settings;
                    div()
                        .id(SharedString::from(format!("nav-{label}")))
                        .h(px(layout::CONTROL_LARGE))
                        .px(px(space::MD))
                        .mb(px(space::XS))
                        .rounded(px(radius::STANDARD))
                        .flex()
                        .items_center()
                        .gap(px(space::MD))
                        .cursor_pointer()
                        .bg(rgb(if active { SURFACE } else { SIDEBAR }))
                        .text_color(rgb(if active { INK } else { MUTED }))
                        .hover(|s| s.bg(rgb(SURFACE)))
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.navigate(page, window, cx)),
                        )
                        .child(icon(
                            symbol,
                            IconSize::Medium,
                            if active { ACCENT } else { MUTED },
                        ))
                        .child(div().flex_1().type_style(Type::Body).child(label))
                        .when(page == Page::Review && !self.reviews.is_empty(), |s| {
                            s.child(
                                div()
                                    .type_style(Type::Caption)
                                    .font_family(font::MONO)
                                    .text_color(rgb(MUTED))
                                    .child(self.reviews.len().to_string()),
                            )
                        })
                }),
            )
            .child(div().flex_1())
            .child(
                div()
                    .id("nav-settings")
                    .text_color(rgb(if self.show_settings { INK } else { MUTED }))
                    .id("nav-settings")
                    .h(px(layout::CONTROL_LARGE))
                    .px(px(space::MD))
                    .rounded(px(radius::STANDARD))
                    .flex()
                    .items_center()
                    .gap(px(space::MD))
                    .cursor_pointer()
                    .when(self.show_settings, |s| s.bg(rgb(SURFACE)))
                    .hover(|s| s.bg(rgb(SURFACE)))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_settings(&OpenSettings, window, cx)
                    }))
                    .child(icon(
                        Icon::Settings,
                        IconSize::Medium,
                        if self.show_settings { ACCENT } else { MUTED },
                    ))
                    .child(div().type_style(Type::Body).child("Settings")),
            )
            .child(
                div()
                    .mt(px(space::LG))
                    .px(px(space::MD))
                    .flex()
                    .items_center()
                    .gap(px(space::SM))
                    .child(div().size(px(5.)).rounded_full().bg(rgb(SUCCESS)))
                    .child(eyebrow("PERSONAL VAULT")),
            )
    }
    pub(super) fn navigate(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.clear_credentials(cx);
        self.page = page;
        self.show_settings = false;
        self.error = None;
        self.notice = None;
        window.focus(&if page == Page::Search {
            self.filter_input.focus_handle(cx)
        } else if page == Page::Browser {
            self.search.focus_handle(cx)
        } else {
            self.focus.clone()
        });
        self.refresh_search(cx);
        if page == Page::Imports {
            self.refresh_imports(cx);
        }
        if page == Page::Browser {
            self.organize_files(cx);
        }
        cx.notify();
    }
    pub(super) fn review_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.selected_reviews.len();
        div()
            .id("review-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(space::PAGE))
            .pb(px(space::PAGE))
            .child(
                div()
                    .w_full()
                    .max_w(px(744.))
                    .mx_auto()
                    .pt(px(space::PAGE_TOP))
                    .flex()
                    .flex_col()
                    .child(heading("Review"))
                    .child(
                        div()
                            .mt(px(space::SM))
                            .mb(px(space::SECTION))
                            .type_style(Type::Body)
                            .text_color(rgb(MUTED))
                            .child(if self.reviews.is_empty() {
                                "You're all caught up."
                            } else {
                                "Confirm these details are correct and belong to you."
                            }),
                    )
                    .when(!self.reviews.is_empty(), |s| {
                        s.child(
                            div()
                                .h(px(layout::CONTROL_LARGE))
                                .mb(px(space::SM))
                                .flex()
                                .items_center()
                                .gap(px(space::LG))
                                .child(
                                    div()
                                        .id("select-reviews")
                                        .cursor_pointer()
                                        .type_style(Type::Small)
                                        .text_color(rgb(MUTED))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if this.selected_reviews.len() == this.reviews.len() {
                                                this.selected_reviews.clear();
                                            } else {
                                                this.selected_reviews = this
                                                    .reviews
                                                    .iter()
                                                    .map(|r| r.id.clone())
                                                    .collect();
                                            }
                                            cx.notify();
                                        }))
                                        .child(if selected == self.reviews.len() {
                                            "Deselect all"
                                        } else {
                                            "Select all"
                                        }),
                                )
                                .child(div().flex_1())
                                .when(selected > 0, |s| {
                                    s.child(
                                        div()
                                            .id("reject-selected")
                                            .type_style(Type::Small)
                                            .text_color(rgb(MUTED))
                                            .cursor_pointer()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.apply_review(false, cx)
                                            }))
                                            .child("Not mine"),
                                    )
                                    .child(
                                        primary_action()
                                            .id("approve-selected")
                                            .h(px(layout::CONTROL_COMPACT))
                                            .hover(|s| s.bg(rgb(PRIMARY_HOVER)))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.apply_review(true, cx)
                                            }))
                                            .child(if self.busy {
                                                "Saving…".into()
                                            } else {
                                                format!("Approve {selected}")
                                            }),
                                    )
                                }),
                        )
                    })
                    .children(self.reviews.iter().map(|entry| {
                        let entry = entry.clone();
                        let key = entry.id.clone();
                        let selected = self.selected_reviews.contains(&key);
                        let item = entry.item;
                        div()
                            .id(ElementId::Name(format!("review-{key}").into()))
                            .py(px(space::LG))
                            .border_b_1()
                            .border_color(rgb(LINE))
                            .flex()
                            .items_start()
                            .gap(px(space::MD))
                            .child(
                                div()
                                    .id(ElementId::Name(format!("select-{key}").into()))
                                    .mt(px(space::MICRO))
                                    .size(px(18.))
                                    .flex_shrink_0()
                                    .border_1()
                                    .border_color(rgb(if selected { ACCENT } else { LINE }))
                                    .rounded(px(radius::STANDARD))
                                    .bg(rgb(if selected { ACCENT } else { SURFACE }))
                                    .cursor_pointer()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if !this.selected_reviews.remove(&key) {
                                            this.selected_reviews.insert(key.clone());
                                        }
                                        cx.notify();
                                    }))
                                    .when(selected, |s| {
                                        s.child(icon(Icon::Check, IconSize::Small, SURFACE))
                                    }),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(space::SM))
                                    .child(
                                        div()
                                            .type_style(Type::Body)
                                            .font_weight(font::MEDIUM)
                                            .child(format!("{} · {}", entry.label, entry.value)),
                                    )
                                    .child(
                                        div()
                                            .type_style(Type::Caption)
                                            .text_color(rgb(MUTED))
                                            .child(if entry.question {
                                                entry.reason
                                            } else {
                                                "Ownership needs confirmation".into()
                                            }),
                                    )
                                    .when(!entry.existing_values.is_empty(), |s| {
                                        s.child(
                                            div()
                                                .type_style(Type::Caption)
                                                .text_color(rgb(WARNING))
                                                .child(format!(
                                                    "Already saved: {}",
                                                    entry.existing_values.join(" · ")
                                                )),
                                        )
                                    })
                                    .child(
                                        div()
                                            .id(ElementId::Name(
                                                format!("source-{}", entry.id).into(),
                                            ))
                                            .type_style(Type::Caption)
                                            .text_color(rgb(ACCENT))
                                            .cursor_pointer()
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.open_document(item, cx)
                                            }))
                                            .child(entry.title),
                                    ),
                            )
                            .child(
                                div()
                                    .id(ElementId::Name(format!("approve-{}", entry.id).into()))
                                    .type_style(Type::Small)
                                    .text_color(rgb(ACCENT))
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.selected_reviews.clear();
                                        this.selected_reviews.insert(entry.id.clone());
                                        this.apply_review(true, cx);
                                    }))
                                    .child("Approve"),
                            )
                    }))
                    .when_some(self.error.clone(), |s, e| {
                        s.child(
                            div()
                                .mt(px(space::LG))
                                .type_style(Type::Small)
                                .text_color(rgb(DANGER))
                                .child(e),
                        )
                    }),
            )
    }
    fn apply_review(&mut self, accept: bool, cx: &mut Context<Self>) {
        if self.busy || self.selected_reviews.is_empty() {
            return;
        }
        let ids = self.selected_reviews.iter().cloned().collect::<Vec<_>>();
        self.run_change(cx, move |vault| {
            vault.review_selected(&ids, accept)?;
            Ok(if accept { "Approved." } else { "Dismissed." }.into())
        });
    }
    pub(super) fn open_document(&mut self, item: u64, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if matches!(
            self.collection.get(item).map(|i| &i.content),
            Some(Content::Credential { .. })
        ) {
            self.open_credential(item, cx);
            return;
        }
        self.document_open = Some(item);
        self.load_proposals(item, cx);
        cx.notify();
    }
    pub(super) fn browser_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).content.to_lowercase();
        let mut root = Tree::default();
        for item in &self.collection.items {
            if !matches!(
                item.content,
                Content::Document { .. } | Content::Credential { .. }
            ) || (!query.is_empty() && !item.title.to_lowercase().contains(&query))
            {
                continue;
            }
            let credential_folder = match &item.content {
                Content::Credential { vault, .. } => Some(format!("Passwords/{vault}")),
                _ => None,
            };
            let path = credential_folder.as_deref().unwrap_or_else(|| {
                self.folders
                    .get(&item.id)
                    .map(String::as_str)
                    .unwrap_or("Unsorted")
            });
            root.insert(path, item.id);
        }
        let rows = root.rows("", 0, &self.expanded_folders, !query.is_empty());
        div()
            .id("browser-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(space::PAGE))
            .pb(px(space::PAGE))
            .child(
                div()
                    .w_full()
                    .max_w(px(744.))
                    .mx_auto()
                    .pt(px(space::PAGE_TOP))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(heading("Browser"))
                            .child(
                                div()
                                    .id("browser-import")
                                    .size(px(30.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| this.pick_documents(cx)))
                                    .child(icon(Icon::Plus, IconSize::Medium, MUTED)),
                            ),
                    )
                    .child(
                        div()
                            .mt(px(space::SM))
                            .mb(px(space::XXL))
                            .type_style(Type::Small)
                            .text_color(rgb(MUTED))
                            .child(if self.organize_cancel.is_some() {
                                "Organizing files…"
                            } else {
                                "Your files, organized for you."
                            }),
                    )
                    .child(
                        div()
                            .id("browser-search")
                            .h(px(layout::CONTROL_LARGE))
                            .px(px(space::MD))
                            .rounded(px(radius::STANDARD))
                            .border_1()
                            .border_color(rgb(LINE))
                            .bg(rgb(SURFACE))
                            .flex()
                            .items_center()
                            .gap(px(space::SM))
                            .on_click(cx.listener(|this, _, window, cx| {
                                window.focus(&this.search.focus_handle(cx))
                            }))
                            .child(icon(Icon::Search, IconSize::Medium, MUTED))
                            .child(self.search.clone()),
                    )
                    .when(rows.is_empty(), |s| {
                        s.child(
                            div()
                                .py(px(space::XXL))
                                .type_style(Type::Body)
                                .text_color(rgb(MUTED))
                                .child(if query.is_empty() {
                                    "Drop a file here to get started."
                                } else {
                                    "No matching files."
                                }),
                        )
                    })
                    .children(rows.into_iter().map(|row| match row {
                        TreeRow::Folder {
                            path,
                            label,
                            depth,
                            count,
                        } => {
                            let expanded =
                                !query.is_empty() || self.expanded_folders.contains(&path);
                            div()
                                .id(ElementId::Name(format!("folder-{path}").into()))
                                .h(px(layout::CONTROL))
                                .pl(px(depth as f32 * layout::TREE_INDENT + space::SM))
                                .pr(px(space::SM))
                                .rounded(px(radius::STANDARD))
                                .flex()
                                .items_center()
                                .gap(px(space::SM))
                                .cursor_pointer()
                                .hover(|s| s.bg(rgb(HOVER)))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if !this.expanded_folders.remove(&path) {
                                        this.expanded_folders.insert(path.clone());
                                    }
                                    cx.notify();
                                }))
                                .child(icon(
                                    if expanded { Icon::Down } else { Icon::Chevron },
                                    IconSize::Small,
                                    FAINT,
                                ))
                                .child(icon(Icon::Folder, IconSize::Medium, ACCENT))
                                .child(div().type_style(Type::Body).child(label))
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .type_style(Type::Caption)
                                        .text_color(rgb(FAINT))
                                        .child(count.to_string()),
                                )
                                .into_any_element()
                        }
                        TreeRow::File { id, depth } => {
                            let item = self
                                .collection
                                .get(id)
                                .expect("tree item is from collection");
                            div()
                                .id(("browser-file", id))
                                .h(px(40.))
                                .pl(px(depth as f32 * layout::TREE_INDENT + space::XXXL))
                                .pr(px(space::SM))
                                .rounded(px(radius::STANDARD))
                                .flex()
                                .items_center()
                                .gap(px(space::SM))
                                .cursor_pointer()
                                .hover(|s| s.bg(rgb(HOVER)))
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.open_document(id, cx)),
                                )
                                .child(icon(Icon::Document, IconSize::Medium, MUTED))
                                .child(
                                    div()
                                        .min_w_0()
                                        .flex_1()
                                        .type_style(Type::Small)
                                        .text_ellipsis()
                                        .child(item.title.clone()),
                                )
                                .child(
                                    div()
                                        .type_style(Type::Caption)
                                        .text_color(rgb(FAINT))
                                        .child(match &item.content {
                                            Content::Document { state, .. } => match state.as_str()
                                            {
                                                "queued" => "Queued",
                                                "running" => "Reading…",
                                                "failed" | "waiting_provider" => "Needs attention",
                                                "needs_review" | "needs_answer" => "Review",
                                                _ => "",
                                            },
                                            _ => "",
                                        }),
                                )
                                .into_any_element()
                        }
                    }))
                    .when_some(self.organization_error.clone(), |s, error| {
                        s.child(
                            div()
                                .mt(px(space::LG))
                                .type_style(Type::Small)
                                .text_color(rgb(MUTED))
                                .child(error)
                                .child(
                                    div()
                                        .id("retry-organization")
                                        .mt(px(space::SM))
                                        .text_color(rgb(ACCENT))
                                        .cursor_pointer()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.organization_attempted = false;
                                            this.organize_files(cx);
                                        }))
                                        .child("Try again"),
                                ),
                        )
                    }),
            )
    }
    pub(super) fn organize_files(&mut self, cx: &mut Context<Self>) {
        if !self.app_ready() || self.organize_cancel.is_some() || self.organization_attempted {
            return;
        }
        let Some(home) = self
            .root
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.join("codex-inbox"))
        else {
            return;
        };
        self.organization_attempted = true;
        self.organization_error = None;
        let cancel = Arc::new(AtomicBool::new(false));
        self.organize_cancel = Some(cancel.clone());
        let session = self.session.clone();
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            loop {
                if cancel.load(Ordering::SeqCst) {
                    return Err("Cancelled.".to_string());
                }
                let input = session
                    .lock()
                    .map_err(|_| "Vault unavailable.")?
                    .as_ref()
                    .ok_or("Vault locked.")?
                    .organization_input()
                    .map_err(|e| e.to_string())?;
                let allowed = input["documents"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|d| d["item"].as_u64())
                    .collect::<Vec<_>>();
                if allowed.is_empty() {
                    return Ok(());
                }
                let folders = me_agent::codex::organize_documents(&home, input, cancel.clone())?;
                if folders.is_empty() {
                    return Err("Some files still need organizing.".into());
                }
                if cancel.load(Ordering::SeqCst) {
                    return Err("Cancelled.".into());
                }
                session
                    .lock()
                    .map_err(|_| "Vault unavailable.")?
                    .as_mut()
                    .ok_or("Vault locked.")?
                    .save_document_folders(&folders, &allowed)
                    .map_err(|e| e.to_string())?;
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.generation {
                    return;
                }
                this.organize_cancel = None;
                if result.is_err() {
                    this.organization_error =
                        Some("Files are safe in Unsorted. Organization couldn't finish.".into());
                }
                this.refresh_search(cx);
                cx.notify();
            });
        })
        .detach();
    }
}

#[derive(Default)]
struct Tree {
    folders: BTreeMap<String, Tree>,
    files: Vec<u64>,
}
enum TreeRow {
    Folder {
        path: String,
        label: String,
        depth: usize,
        count: usize,
    },
    File {
        id: u64,
        depth: usize,
    },
}
impl Tree {
    fn insert(&mut self, path: &str, id: u64) {
        let mut node = self;
        for part in path.split('/') {
            node = node.folders.entry(part.into()).or_default();
        }
        node.files.push(id);
    }
    fn count(&self) -> usize {
        self.files.len() + self.folders.values().map(Self::count).sum::<usize>()
    }
    fn rows(
        &self,
        parent: &str,
        depth: usize,
        expanded: &BTreeSet<String>,
        searching: bool,
    ) -> Vec<TreeRow> {
        let mut rows = Vec::new();
        for (label, node) in &self.folders {
            let path = if parent.is_empty() {
                label.clone()
            } else {
                format!("{parent}/{label}")
            };
            rows.push(TreeRow::Folder {
                path: path.clone(),
                label: label.clone(),
                depth,
                count: node.count(),
            });
            if searching || expanded.contains(&path) {
                rows.extend(node.rows(&path, depth + 1, expanded, searching));
            }
        }
        rows.extend(self.files.iter().map(|&id| TreeRow::File { id, depth }));
        rows
    }
}
