use super::*;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

#[derive(Default)]
pub(super) struct FilterState {
    pub query: String,
    pub revision: u64,
    pub cancel: Option<Arc<AtomicBool>>,
    pub facts: Vec<me_core::DataFact>,
    pub items: Vec<me_core::Item>,
    pub local_revision: u64,
    pub local_loading: bool,
    pub local_error: Option<String>,
    pub keys: Vec<String>,
    pub loading: bool,
    pub error: Option<String>,
    pub copied: Option<String>,
    pub form_revision: u64,
    pub form_cancel: Option<Arc<AtomicBool>>,
    pub form_loading: bool,
    pub form_needs: Vec<me_core::FormNeed>,
    pub form_checked: bool,
    pub handoff_review: bool,
    pub handoff_busy: bool,
    pub handoff_ready: Option<PathBuf>,
}
impl MeApp {
    pub(super) fn stop_filter(&mut self) {
        for cancel in [
            &self.filter.cancel,
            &self.filter.form_cancel,
            &self.organize_cancel,
        ]
        .into_iter()
        .flatten()
        {
            cancel.store(true, Ordering::SeqCst);
        }
        self.filter.revision += 1;
        self.filter.local_revision += 1;
        self.filter.local_loading = false;
        self.filter.cancel = None;
        self.filter.form_cancel = None;
        self.filter.loading = false;
        self.filter.form_loading = false;
    }
    fn matches(&self) -> Vec<me_core::DataFact> {
        me_core::filter_data(&self.filter.facts, &self.filter.query, &self.filter.keys)
    }
    pub(super) fn filter_changed(&mut self, cx: &mut Context<Self>) {
        let query = self.filter_input.read(cx).content.trim().to_string();
        if query == self.filter.query {
            cx.notify();
            return;
        }
        if let Some(cancel) = self.filter.cancel.take() {
            cancel.store(true, Ordering::SeqCst);
        }
        self.filter.query = query;
        self.filter.keys.clear();
        self.filter.error = None;
        self.filter.revision += 1;
        self.refresh_local_search(cx);
        self.filter.loading = self.ai_ready() && !self.filter.query.is_empty();
        let revision = self.filter.revision;
        if self.ai_ready() && !self.filter.query.is_empty() {
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(650))
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if this.filter.revision == revision {
                        this.start_semantic_filter(cx);
                    }
                });
            })
            .detach();
        }
        cx.notify();
    }
    /// Search every collection kind locally. Credential payloads never enter the
    /// field catalog or the AI filter; collection search reads their metadata only.
    pub(super) fn refresh_local_search(&mut self, cx: &mut Context<Self>) {
        self.filter.local_revision += 1;
        self.filter.items.clear();
        self.filter.local_error = None;
        self.filter.local_loading = false;
        if !self.app_ready() || !self.filter.query.chars().any(char::is_alphanumeric) {
            return;
        }
        let revision = self.filter.local_revision;
        let generation = self.generation;
        self.filter.local_loading = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(120))
                .await;
            let request = this.update(cx, |this, _| {
                (this.generation == generation && this.filter.local_revision == revision)
                    .then(|| (this.session.clone(), this.filter.query.clone()))
            });
            let Ok(Some((session, query))) = request else {
                return;
            };
            let result = cx
                .background_executor()
                .spawn(async move {
                    let guard = session.lock().map_err(|_| me_core::Error::Format)?;
                    let vault = guard.as_ref().ok_or(me_core::Error::Format)?;
                    vault
                        .collection(&query, false)
                        .map(|collection| collection.items)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation || this.filter.local_revision != revision {
                    return;
                }
                this.filter.local_loading = false;
                match result {
                    Ok(items) => this.filter.items = items,
                    Err(error) => this.filter.local_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn submit_filter(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.filter.query.is_empty() {
            return;
        }
        self.filter.revision += 1;
        self.start_semantic_filter(cx);
    }
    fn start_semantic_filter(&mut self, cx: &mut Context<Self>) {
        if !self.ai_ready() || self.filter.query.is_empty() {
            self.filter.loading = false;
            cx.notify();
            return;
        }
        if let Some(cancel) = self.filter.cancel.take() {
            cancel.store(true, Ordering::SeqCst);
        }
        let Some(home) = self
            .root
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.join("codex-inbox"))
        else {
            return;
        };
        let query = self.filter.query.clone();
        if query.len() > 4000 {
            self.filter.error = Some("Use a shorter search.".into());
            self.filter.loading = false;
            cx.notify();
            return;
        }
        let catalog = me_core::field_catalog(&self.filter.facts);
        if catalog.is_empty() {
            self.filter.loading = false;
            cx.notify();
            return;
        }
        let revision = self.filter.revision;
        let generation = self.generation;
        let cancel = Arc::new(AtomicBool::new(false));
        self.filter.cancel = Some(cancel.clone());
        self.filter.loading = true;
        let task = cx
            .background_executor()
            .spawn(async move { me_agent::codex::filter_fields(&home, &query, &catalog, cancel) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation || this.filter.revision != revision {
                    return;
                }
                this.filter.cancel = None;
                this.filter.loading = false;
                match result {
                    Ok(keys) => {
                        this.filter.keys = keys;
                        this.remember_matches(cx);
                    }
                    Err(_) => {
                        this.filter.error =
                            Some("Showing local matches. Retry AI search with Enter.".into())
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn remember_matches(&mut self, cx: &mut Context<Self>) {
        let ids = self
            .matches()
            .into_iter()
            .take(12)
            .map(|f| f.id)
            .collect::<Vec<_>>();
        self.remember_details(ids, cx);
    }
    fn remember_details(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        if ids.is_empty() {
            return;
        }
        let session = self.session.clone();
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            let mut guard = session.lock().map_err(|_| me_core::Error::Format)?;
            let vault = guard.as_mut().ok_or(me_core::Error::Format)?;
            vault.remember_data(&ids)?;
            vault.data_facts()
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    if let Ok(facts) = result {
                        this.filter.facts = facts;
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }
    fn copy_detail(&mut self, fact: me_core::DataFact, cx: &mut Context<Self>) {
        if !self.app_ready() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(fact.value.clone()));
        self.copied_value = Some(zeroize::Zeroizing::new(fact.value));
        self.clear_clipboard_later(cx);
        self.filter.copied = Some(fact.id.clone());
        self.remember_details(vec![fact.id], cx);
        cx.notify();
    }
    fn data_row(
        &self,
        fact: &me_core::DataFact,
        context: Option<&str>,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let item = fact.item;
        let copied = self.filter.copied.as_ref() == Some(&fact.id);
        let copy = fact.clone();
        div()
            .px(px(space::XL))
            .py(px(space::LG))
            .bg(rgb(SURFACE))
            .border_1()
            .border_color(rgb(LINE))
            .rounded(px(radius::STANDARD))
            .flex()
            .items_center()
            .gap(px(space::LG))
            .child(
                div()
                    .size(px(34.))
                    .rounded(px(radius::STANDARD))
                    .bg(rgb(HOVER))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(Icon::Hash, IconSize::Medium, MUTED)),
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
                            .flex()
                            .items_center()
                            .gap(px(space::SM))
                            .child(
                                div()
                                    .type_style(Type::Caption)
                                    .text_color(rgb(MUTED))
                                    .text_ellipsis()
                                    .child(context.unwrap_or(&fact.label).to_string()),
                            )
                            .when(fact.conflict, |s| {
                                s.child(
                                    div()
                                        .type_style(Type::Caption)
                                        .text_color(rgb(WARNING))
                                        .child("Multiple values"),
                                )
                            }),
                    )
                    .child(
                        div()
                            .type_style(Type::Value)
                            .font_family(font::MONO)
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(fact.value.clone()),
                    )
                    .child(
                        div()
                            .id(gpui::SharedString::from(format!(
                                "data-source-{}-{}",
                                context.unwrap_or(""),
                                fact.id
                            )))
                            .type_style(Type::Caption)
                            .text_color(rgb(FAINT))
                            .cursor_pointer()
                            .text_ellipsis()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if matches!(
                                    this.collection.get(item).map(|i| &i.content),
                                    Some(Content::Note(_))
                                ) {
                                    this.open_editor(Some(item), window, cx);
                                } else {
                                    this.open_document(item, cx);
                                }
                            }))
                            .child(fact.source.clone()),
                    ),
            )
            .child(
                div()
                    .id(gpui::SharedString::from(format!(
                        "copy-data-{}-{}",
                        context.unwrap_or(""),
                        fact.id
                    )))
                    .h(px(layout::CONTROL_COMPACT))
                    .px(px(space::SM))
                    .rounded(px(radius::STANDARD))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(space::SM))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(HOVER)))
                    .on_click(cx.listener(move |this, _, _, cx| this.copy_detail(copy.clone(), cx)))
                    .child(icon(
                        if copied { Icon::Check } else { Icon::Copy },
                        IconSize::Medium,
                        if copied { ACCENT } else { MUTED },
                    ))
                    .child(
                        div()
                            .type_style(Type::Caption)
                            .text_color(rgb(MUTED))
                            .child(if copied { "Copied" } else { "Copy" }),
                    ),
            )
    }
    fn search_item_row(
        &self,
        item: &me_core::Item,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let id = item.id;
        let note = matches!(item.content, Content::Note(_));
        let (symbol, description) = match &item.content {
            Content::Note(_) => (Icon::Text, "Note".to_string()),
            Content::Document { extension, .. } => (
                Icon::Document,
                format!("Document · {}", extension.to_uppercase()),
            ),
            Content::Credential {
                category,
                vault,
                archived,
            } => (
                Icon::Fingerprint,
                format!(
                    "{} · {}{}",
                    me_core::credential_category(category),
                    vault,
                    if *archived { " · Archived" } else { "" }
                ),
            ),
        };
        div()
            .id(("search-item", id))
            .px(px(space::XL))
            .py(px(space::LG))
            .bg(rgb(SURFACE))
            .border_1()
            .border_color(rgb(LINE))
            .rounded(px(radius::STANDARD))
            .flex()
            .items_center()
            .gap(px(space::LG))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(HOVER)))
            .on_click(cx.listener(move |this, _, window, cx| {
                if note {
                    this.open_editor(Some(id), window, cx);
                } else {
                    this.open_document(id, cx);
                }
            }))
            .child(icon(symbol, IconSize::Large, MUTED))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(space::XS))
                    .child(
                        div()
                            .type_style(Type::Label)
                            .text_ellipsis()
                            .child(item.title.clone()),
                    )
                    .child(
                        div()
                            .type_style(Type::Caption)
                            .text_color(rgb(MUTED))
                            .text_ellipsis()
                            .child(description),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap(px(space::SM))
                    .type_style(Type::Small)
                    .text_color(rgb(ACCENT))
                    .child("Open")
                    .child(icon(Icon::Chevron, IconSize::Medium, MUTED)),
            )
    }
    pub(super) fn filter_view(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let searching = !self.filter.query.is_empty();
        let form = !searching && !self.attachments.is_empty();
        let recent = self.filter.facts.iter().any(|f| f.recent > 0);
        let rows = if searching {
            self.matches()
        } else {
            self.filter
                .facts
                .iter()
                .filter(|f| !recent || f.recent > 0)
                .take(8)
                .cloned()
                .collect()
        };
        // A note already shown as a copyable fact needs only one result row.
        let items: Vec<_> = self
            .filter
            .items
            .iter()
            .filter(|item| {
                searching
                    && !(matches!(item.content, Content::Note(_))
                        && rows.iter().any(|fact| fact.item == item.id))
            })
            .collect();
        let result_count = rows.len() + items.len();
        let status = if self.busy {
            Some("Saving files…".to_string())
        } else if self.filter.form_loading {
            Some("Checking the form…".into())
        } else if self.filter.local_loading {
            Some("Searching your vault…".into())
        } else if self.filter.local_error.is_some() {
            self.filter.local_error.clone()
        } else if self.filter.loading {
            Some("Finding matching fields…".into())
        } else {
            self.filter.error.clone().or(self.error.clone())
        };
        div()
            .id("filter-page")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(space::PAGE))
            .pt(px(space::PAGE_TOP))
            .pb(px(space::PAGE))
            .child(
                div()
                    .max_w(px(layout::SEARCH_WIDTH))
                    .mx_auto()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .mb(px(space::SECTION))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(space::SM))
                                    .child(eyebrow("PERSONAL SEARCH"))
                                    .child(
                                        div()
                                            .type_style(Type::Hero)
                                            .font_weight(font::MEDIUM)
                                            .child("It's about you."),
                                    )
                                    .child(
                                        div()
                                            .type_style(Type::Label)
                                            .text_color(rgb(MUTED))
                                            .child("Your data. Your fingerprints."),
                                    ),
                            )
                            .child(icon(Icon::Fingerprint, IconSize::Hero, DECORATIVE)),
                    )
                    .child(
                        div()
                            .rounded(px(radius::STANDARD))
                            .border_1()
                            .border_color(rgb(
                                if self.filter_input.focus_handle(cx).is_focused(window) {
                                    FOCUS
                                } else {
                                    LINE
                                },
                            ))
                            .bg(rgb(SURFACE))
                            .p(px(space::LG))
                            .flex()
                            .flex_col()
                            .gap(px(space::LG))
                            .when(!self.attachments.is_empty(), |s| {
                                s.child(
                                    div().flex().flex_wrap().gap(px(space::SM)).children(
                                        self.attachments
                                            .iter()
                                            .filter_map(|id| self.collection.get(*id))
                                            .map(|item| {
                                                let id = item.id;
                                                div()
                                                    .rounded(px(radius::STANDARD))
                                                    .bg(rgb(HOVER))
                                                    .px(px(space::SM))
                                                    .py(px(space::SM))
                                                    .flex()
                                                    .items_center()
                                                    .gap(px(space::SM))
                                                    .child(icon(
                                                        Icon::Document,
                                                        IconSize::Small,
                                                        MUTED,
                                                    ))
                                                    .child(
                                                        div()
                                                            .type_style(Type::Caption)
                                                            .child(item.title.clone()),
                                                    )
                                                    .child(
                                                        div()
                                                            .id(("remove-form", id))
                                                            .cursor_pointer()
                                                            .on_click(cx.listener(
                                                                move |this, _, _, cx| {
                                                                    this.attachments
                                                                        .retain(|i| *i != id);
                                                                    this.inspect_attached_forms(cx);
                                                                },
                                                            ))
                                                            .child(icon(
                                                                Icon::Close,
                                                                IconSize::Small,
                                                                MUTED,
                                                            )),
                                                    )
                                            }),
                                    ),
                                )
                            })
                            .child(
                                div()
                                    .h(px(30.))
                                    .flex()
                                    .items_center()
                                    .gap(px(space::MD))
                                    .child(icon(Icon::Search, IconSize::Large, ACCENT))
                                    .child(
                                        div().flex_1().min_w_0().child(self.filter_input.clone()),
                                    )
                                    .when(searching, |s| {
                                        s.child(
                                            div()
                                                .id("clear-filter")
                                                .cursor_pointer()
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.filter_input.update(cx, |input, cx| {
                                                        input.set_text("", cx)
                                                    });
                                                    window
                                                        .focus(&this.filter_input.focus_handle(cx));
                                                }))
                                                .child(icon(Icon::Close, IconSize::Medium, MUTED)),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .id("filter-upload")
                                            .flex()
                                            .items_center()
                                            .gap(px(space::SM))
                                            .cursor_pointer()
                                            .on_click(
                                                cx.listener(|this, _, _, cx| {
                                                    this.pick_documents(cx)
                                                }),
                                            )
                                            .child(icon(Icon::Plus, IconSize::Small, MUTED))
                                            .child(
                                                div()
                                                    .type_style(Type::Caption)
                                                    .text_color(rgb(MUTED))
                                                    .child("Add a file"),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .font_family(font::MONO)
                                            .type_style(Type::Caption)
                                            .text_color(rgb(FAINT))
                                            .child("SEARCH ALL  /  ⌘K"),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .h(px(34.))
                            .pt(px(space::SM))
                            .when_some(status, |s, text| {
                                s.child(
                                    div()
                                        .type_style(Type::Caption)
                                        .text_color(rgb(MUTED))
                                        .child(text),
                                )
                            }),
                    )
                    .child(
                        div()
                            .mt(px(space::SM))
                            .mb(px(space::MD))
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(eyebrow(if searching {
                                "SEARCH RESULTS"
                            } else if form {
                                "FORM FIELDS"
                            } else if recent {
                                "RECENTLY FOUND"
                            } else {
                                "YOUR DATA"
                            }))
                            .child(
                                div()
                                    .type_style(Type::Caption)
                                    .font_family(font::MONO)
                                    .text_color(rgb(FAINT))
                                    .child(
                                        if form {
                                            self.filter.form_needs.len()
                                        } else {
                                            result_count
                                        }
                                        .to_string(),
                                    ),
                            ),
                    )
                    .when(!form, |s| {
                        s.child(
                            div().flex().flex_col().gap(px(space::SM))
                            .children(items.iter().take(60).map(|item| self.search_item_row(item, cx)))
                            .children(
                                rows.iter()
                                    .take(60)
                                    .map(|fact| self.data_row(fact, None, cx)),
                            ),
                        )
                        .when(result_count == 0 && !self.filter.local_loading && self.filter.local_error.is_none(), |s| {
                            s.child(
                                div()
                                    .py(px(space::XXL))
                                    .type_style(Type::Body)
                                    .text_color(rgb(MUTED))
                                    .child(if searching {
                                        "No matching items or details."
                                    } else {
                                        "Add a file or a detail to get started."
                                    }),
                            )
                        })
                        .when(rows.len() > 60 || items.len() > 60, |s| {
                            s.child(
                                div()
                                    .mt(px(space::MD))
                                    .type_style(Type::Caption)
                                    .text_color(rgb(MUTED))
                                    .child("Showing up to 60 items and 60 details. Refine your search."),
                            )
                        })
                    })
                    .when(form, |s| {
                        s.child(
                            div().flex().flex_col().gap(px(space::SM)).children(
                                self.filter
                                    .form_needs
                                    .iter()
                                    .enumerate()
                                    .map(|(index, need)| {
                                        let matches = if need.keys.is_empty() {
                                            vec![]
                                        } else {
                                            me_core::filter_data(&self.filter.facts, "", &need.keys)
                                        };
                                        div()
                                            .id(("form-field", index))
                                            .flex()
                                            .flex_col()
                                            .gap(px(space::SM))
                                            .when(matches.is_empty(), |s| {
                                                s.child(
                                                    div()
                                                        .px(px(space::XL))
                                                        .py(px(space::LG))
                                                        .border_1()
                                                        .border_color(rgb(LINE))
                                                        .rounded(px(radius::STANDARD))
                                                        .flex()
                                                        .items_center()
                                                        .justify_between()
                                                        .child(
                                                            div()
                                                                .type_style(Type::Body)
                                                                .child(need.label.clone()),
                                                        )
                                                        .child(
                                                            div()
                                                                .type_style(Type::Caption)
                                                                .text_color(rgb(MUTED))
                                                                .child("Not found"),
                                                        ),
                                                )
                                            })
                                            .children(matches.iter().map(|fact| {
                                                self.data_row(fact, Some(&need.label), cx)
                                            }))
                                    }),
                            ),
                        )
                        .when(
                            self.filter.form_checked && self.filter.form_needs.is_empty(),
                            |s| {
                                s.child(
                                    div()
                                        .py(px(space::XL))
                                        .type_style(Type::Body)
                                        .text_color(rgb(MUTED))
                                        .child("No empty fields found in this file."),
                                )
                            },
                        )
                        .when(
                            self.filter.form_checked && !self.filter.form_needs.is_empty(),
                            |s| {
                                s.child(
                                    div()
                                        .mt(px(space::LG))
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            div()
                                                .type_style(Type::Small)
                                                .text_color(rgb(MUTED))
                                                .child("Fill a copy with Codex."),
                                        )
                                        .child(
                                            primary_action()
                                                .id("review-handoff")
                                                .hover(|s| s.bg(rgb(PRIMARY_HOVER)))
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.filter.handoff_review = true;
                                                    cx.notify();
                                                }))
                                                .child("Review handoff")
                                                .child(icon(
                                                    Icon::Arrow,
                                                    IconSize::Medium,
                                                    SURFACE,
                                                )),
                                        ),
                                )
                            },
                        )
                        .when(
                            !self.filter.form_loading && !self.filter.form_checked,
                            |s| {
                                s.child(
                                    div()
                                        .id("retry-form")
                                        .mt(px(space::MD))
                                        .type_style(Type::Small)
                                        .text_color(rgb(ACCENT))
                                        .cursor_pointer()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.inspect_attached_forms(cx)
                                        }))
                                        .child("Retry form check"),
                                )
                            },
                        )
                    })
                    .when(!searching && !form, |s| {
                        s.child(
                            div()
                                .mt(px(space::LG))
                                .id("add-home-detail")
                                .type_style(Type::Caption)
                                .text_color(rgb(MUTED))
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_editor(None, window, cx)
                                }))
                                .child("+ Add a detail"),
                        )
                    })
                    .when_some(self.filter.handoff_ready.as_ref(), |s, path| {
                        s.child(
                            div()
                                .mt(px(space::LG))
                                .type_style(Type::Small)
                                .text_color(rgb(MUTED))
                                .child(format!(
                                    "Handoff saved in {}. Paste the copied task in Codex to start.",
                                    path.display()
                                )),
                        )
                    }),
            )
    }
    pub(super) fn inspect_attached_forms(&mut self, cx: &mut Context<Self>) {
        if let Some(cancel) = self.filter.form_cancel.take() {
            cancel.store(true, Ordering::SeqCst);
        }
        self.filter.form_revision += 1;
        let form_revision = self.filter.form_revision;
        self.filter.form_needs.clear();
        self.filter.form_checked = false;
        self.filter.error = None;
        self.filter.handoff_ready = None;
        if !self.ai_ready() || self.attachments.is_empty() {
            self.filter.form_loading = false;
            cx.notify();
            return;
        }
        if self.attachments.len() > 8 {
            self.filter.error = Some("Check up to eight forms at once.".into());
            self.filter.form_loading = false;
            cx.notify();
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
        let items = self.attachments.clone();
        let expected = items.clone();
        let session = self.session.clone();
        let generation = self.generation;
        let cancel = Arc::new(AtomicBool::new(false));
        self.filter.form_cancel = Some(cancel.clone());
        self.filter.form_loading = true;
        let task = cx.background_executor().spawn(async move {
            let mut combined = String::new();
            for item in &items {
                if cancel.load(Ordering::SeqCst) {
                    return Err("Cancelled.".to_string());
                }
                let input = {
                    let mut guard = session.lock().map_err(|_| "Vault unavailable.")?;
                    let vault = guard.as_mut().ok_or("Vault locked.")?;
                    vault
                        .begin_evaluation(*item, false)
                        .map_err(|e| e.to_string())?;
                    match vault.begin_document_processing(*item) {
                        Ok(input) => input,
                        Err(error) => {
                            let message = error.to_string();
                            let _ = vault.finish_evaluation(*item, Some(&message));
                            return Err(message);
                        }
                    }
                };
                if let Some(input) = input {
                    let parts = me_documents::extract(&input.extension, &input.bytes, &cancel);
                    let mut guard = session.lock().map_err(|_| "Vault unavailable.")?;
                    let vault = guard.as_mut().ok_or("Vault locked.")?;
                    match parts {
                        Ok(parts) => {
                            if let Err(error) = vault.finish_document_processing(&input, &parts) {
                                let message = error.to_string();
                                let _ = vault.fail_document_processing(&input);
                                let _ = vault.finish_evaluation(*item, Some(&message));
                                return Err(message);
                            }
                        }
                        Err(error) => {
                            let _ = vault.fail_document_processing(&input);
                            let _ = vault.finish_evaluation(*item, Some(&error));
                            return Err(error);
                        }
                    }
                }
                let text = {
                    let mut guard = session.lock().map_err(|_| "Vault unavailable.")?;
                    let vault = guard.as_mut().ok_or("Vault locked.")?;
                    vault
                        .finish_evaluation(*item, None)
                        .map_err(|e| e.to_string())?;
                    vault.form_text(*item).map_err(|e| e.to_string())?
                };
                combined.push_str(&text);
                combined.push('\n');
            }
            if cancel.load(Ordering::SeqCst) {
                return Err("Cancelled.".into());
            }
            let catalog = {
                let guard = session.lock().map_err(|_| "Vault unavailable.")?;
                let vault = guard.as_ref().ok_or("Vault locked.")?;
                me_core::field_catalog(&vault.data_facts().map_err(|e| e.to_string())?)
            };
            me_agent::codex::inspect_form(&home, &combined, &catalog, cancel)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation
                    || this.attachments != expected
                    || this.filter.form_revision != form_revision
                {
                    return;
                }
                this.filter.form_cancel = None;
                this.filter.form_loading = false;
                match result {
                    Ok(needs) => {
                        this.filter.form_needs = needs;
                        this.filter.form_checked = true;
                    }
                    Err(error) => this.filter.error = Some(error),
                }
                this.refresh_imports(cx);
                this.refresh_search(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn handoff_modal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let available = self
            .filter
            .form_needs
            .iter()
            .filter(|n| {
                let rows = me_core::filter_data(&self.filter.facts, "", &n.keys);
                !n.keys.is_empty() && rows.len() == 1 && !rows[0].conflict
            })
            .count();
        self.overlay(cx).child(modal_panel(500.).max_h(gpui::relative(0.85))
            .child(heading("Continue in Codex"))
            .child(div().type_style(Type::Body).child(format!("Share {} form(s) and {} matched details. Missing or conflicting values stay blank.",self.attachments.len(),available)))
            .child(div().id("handoff-fields").max_h(px(210.)).overflow_y_scroll().flex().flex_col().gap(px(space::SM)).children(self.filter.form_needs.iter().map(|need| {
                let matches=me_core::filter_data(&self.filter.facts,"",&need.keys);
                let value=if !need.keys.is_empty() && matches.len()==1 && !matches[0].conflict {matches[0].value.clone()}else{"Needs your input".into()};
                div().flex().justify_between().gap(px(space::MD)).child(div().type_style(Type::Small).text_color(rgb(MUTED)).child(need.label.clone())).child(div().type_style(Type::Small).font_family(font::MONO).text_ellipsis().child(value))
            })))
            .child(div().type_style(Type::Small).text_color(rgb(MUTED)).child("Choose where to save an unencrypted working copy. ME. opens the folder in Codex and copies the task. Paste it there to start; review the filled copy before signing or sending."))
            .child(div().flex().justify_end().gap(px(space::LG))
                .child(div().id("cancel-handoff").px(px(space::MD)).py(px(space::SM)).type_style(Type::Small).cursor_pointer().on_click(cx.listener(|this,_,_,cx|{this.filter.handoff_review=false;cx.notify();})).child("Cancel"))
                .child(primary_action().id("approve-handoff").hover(|s| s.bg(rgb(PRIMARY_HOVER))).on_click(cx.listener(|this,_,_,cx|this.prepare_handoff(cx))).child(if self.filter.handoff_busy{"Preparing…"}else{"Choose folder & open Codex"}))))
    }
    fn prepare_handoff(&mut self, cx: &mut Context<Self>) {
        if self.filter.handoff_busy {
            return;
        }
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Save handoff here".into()),
        });
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let picked = picker.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                if let Ok(Ok(Some(paths))) = picked
                    && let Some(parent) = paths.first()
                {
                    this.export_handoff(parent.clone(), cx);
                }
            });
        })
        .detach();
    }
    fn export_handoff(&mut self, parent: PathBuf, cx: &mut Context<Self>) {
        self.filter.handoff_busy = true;
        let session = self.session.clone();
        let items = self.attachments.clone();
        let needs = self.filter.form_needs.clone();
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            let guard = session
                .lock()
                .map_err(|_| "Vault unavailable.".to_string())?;
            let vault = guard.as_ref().ok_or("Vault locked.")?;
            vault
                .export_form_handoff(&items, &needs, &parent)
                .map_err(|e| e.to_string())
        });
        cx.spawn(async move|this,cx|{let result=task.await;let _=this.update(cx,|this,cx|{
            if this.generation!=generation{return;}
            this.filter.handoff_busy=false;this.filter.handoff_review=false;
            match result {
                Ok(path)=>{
                    let prompt="Fill a copy of the form in this folder. Read HANDOFF.md and matched-data.json. Use computer use if needed, ask about missing fields, and return the completed file for my review.";
                    cx.write_to_clipboard(ClipboardItem::new_string(prompt.into()));
                    this.filter.handoff_ready=Some(path.clone());
                    let open=cx.background_executor().spawn(async move{me_agent::codex::open_form_workspace(&path)});
                    cx.spawn(async move|this,cx|{let result=open.await;let _=this.update(cx,|this,cx|{if this.generation==generation {if let Err(error)=result{this.filter.error=Some(error);}cx.notify();}});}).detach();
                },
                Err(error)=>this.filter.error=Some(error),
            }cx.notify();
        });}).detach();
        cx.notify();
    }
}
