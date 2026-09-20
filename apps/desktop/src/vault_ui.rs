use super::*;
use std::time::Duration;
use zeroize::Zeroizing;

impl MeApp {
    pub(super) fn vault_path() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("ME_VAULT_DIR") {
            let path = PathBuf::from(path);
            return path.is_absolute().then_some(path);
        }
        let home = PathBuf::from(std::env::var_os("HOME")?);
        let base = if cfg!(target_os = "macos") {
            home.join("Library/Application Support")
        } else {
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home.join(".local/share"))
        };
        Some(base.join("ME/vault"))
    }

    pub(super) fn inspect_vault(cx: &mut Context<Self>) {
        let root = Self::vault_path();
        let task = cx.background_executor().spawn(async move {
            root.as_deref()
                .ok_or(me_core::Error::Validation("Couldn't locate the vault."))
                .and_then(Vault::exists)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(exists) => this.initialized = exists,
                    Err(err) => this.error = Some(err.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.on_app_quit(|this, cx| {
            this.stop_filter();
            this.stop_ai();
            if let Some(cancel) = &this.intake_cancel {
                cancel.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            this.stop_codex_setup();
            this.stop_bridge(cx);
            this.clear_owned_clipboard(cx);
            let session = this.session.clone();
            let view = this.knowledge.loaded.then(|| this.knowledge.view.clone());
            cx.background_executor().spawn(async move {
                if let Ok(mut guard) = session.lock() {
                    if let (Some(vault), Some(view)) = (guard.as_mut(), view) {
                        let _ = vault.save_knowledge_viewport(&view);
                    }
                    guard.take();
                }
            })
        })
        .detach();
    }

    pub(super) fn unlock_vault(&mut self, cx: &mut Context<Self>) {
        if self.unlocked || self.busy {
            return;
        }
        let Some(root) = self.root.clone() else {
            return;
        };
        let password = Zeroizing::new(self.password.read(cx).content.to_string());
        if !self.initialized
            && self.restore_from.is_none()
            && password.as_str() != self.password_repeat.read(cx).content.as_ref()
        {
            self.error = Some("Passwords don't match.".into());
            cx.notify();
            return;
        }
        self.password.update(cx, |i, cx| i.set_text("", cx));
        self.password_repeat.update(cx, |i, cx| i.set_text("", cx));
        self.busy = true;
        self.error = None;
        let initialized = self.initialized;
        let restore = self.restore_from.clone();
        let session = self.session.clone();
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            let vault = if let Some(backup) = restore {
                Vault::restore(&backup, &root, &password)
            } else if initialized {
                Vault::unlock(&root, &password)
            } else {
                Vault::create(&root, &password)
            }?;
            let collection = vault.collection("", false)?;
            let settings = vault.settings()?;
            *session.lock().map_err(|_| me_core::Error::Format)? = Some(vault);
            Ok::<_, me_core::Error>((collection, settings))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.busy = false;
                match result {
                    Ok((collection, settings)) => {
                        this.settings = settings;
                        this.collection = collection;
                        this.unlocked = true;
                        this.initialized = true;
                        this.restore_from = None;
                        this.notice = None;
                        this.focus_filter_on_ready = true;
                        this.refresh_search(cx);
                        this.refresh_imports(cx);
                        this.kick_auto_queue(cx);
                    }
                    Err(error) => {
                        this.error = Some(error.to_string());
                        this.focus_password_on_ready = true;
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn lock_vault(
        &mut self,
        _: &LockVault,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(any(debug_assertions, feature = "development-tools"))]
        if self.development.wiping {
            return;
        }
        let knowledge_view = self.knowledge.loaded.then(|| self.knowledge.view.clone());
        let old = self.detach_vault(cx);
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            if let Ok(mut guard) = old.lock() {
                if let (Some(vault), Some(view)) = (guard.as_mut(), knowledge_view) {
                    let _ = vault.save_knowledge_viewport(&view);
                }
                guard.take();
            }
        });
        cx.spawn(async move |this, cx| {
            task.await;
            let _ = this.update(cx, |this, cx| {
                if generation == this.generation {
                    this.busy = false;
                    cx.notify();
                }
            });
        })
        .detach();
        window.focus(&self.password.focus_handle(cx));
        cx.notify();
    }

    /// Invalidate callbacks, cancel workers, and disconnect their session before
    /// closing or resetting storage. Old workers can never access a new session.
    pub(super) fn detach_vault(&mut self, cx: &mut Context<Self>) -> Arc<Mutex<Option<Vault>>> {
        #[cfg(any(debug_assertions, feature = "development-tools"))]
        {
            self.development.confirming = false;
        }
        self.stop_filter();
        self.filter = FilterState::default();
        self.knowledge = KnowledgeState::default();
        self.attachments.clear();
        self.reviews.clear();
        self.selected_reviews.clear();
        self.folders.clear();
        self.expanded_folders.clear();
        self.page = Page::Search;
        self.focus_filter_on_ready = true;
        self.organization_attempted = false;
        self.organization_error = None;
        self.generation += 1;
        self.clear_credentials(cx);
        self.stop_ai();
        self.stop_bridge(cx);
        self.proposals.clear();
        self.questions.clear();
        self.question_edit = None;
        self.question_error = None;
        self.ai_message = None;
        self.ai_failure = None;
        self.ai_item = None;
        self.active_imports.clear();
        self.import_jobs.clear();
        self.expanded_imports.clear();
        self.import_pause = None;
        self.import_refresh = false;
        self.import_refresh_pending = false;
        self.pending_imports.clear();
        self.pending_attach_to_search = false;
        self.import_scans = 0;
        self.pending_generation += 1;
        self.intake_active = false;
        if let Some(cancel) = self.intake_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        self.intake_files.clear();
        self.intake_message = None;
        self.intake_failures.clear();
        self.auto_queue_check = false;
        self.settings_saving = false;
        self.show_settings = false;
        self.document_error = None;
        self.document_warning = None;
        self.ai_warning = None;
        self.search_generation += 1;
        self.unlocked = false;
        self.show_codex_setup = false;
        self.focus_password_on_ready = true;
        // A browser sign-in is explicit session work. The passive startup check may
        // finish while locked; its result is only surfaced after the next unlock.
        if self.codex_login_requested {
            self.stop_codex_setup();
        }
        self.busy = true;
        self.collection = Collection::default();
        self.history.clear();
        self.editing = None;
        self.show_add = false;
        self.show_info = false;
        self.document_open = None;
        self.pinned_only = false;
        self.notice = None;
        self.error = None;
        self.clear_owned_clipboard(cx);
        for field in [
            &self.search,
            &self.filter_input,
            &self.knowledge_input,
            &self.title_input,
            &self.value_input,
            &self.password,
            &self.password_repeat,
        ] {
            field.update(cx, |i, cx| i.set_text("", cx));
        }
        std::mem::replace(&mut self.session, Arc::new(Mutex::new(None)))
    }

    pub(super) fn run_change(
        &mut self,
        cx: &mut Context<Self>,
        operation: impl FnOnce(&mut Vault) -> me_core::Result<String> + Send + 'static,
    ) {
        if !self.app_ready() || self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let session = self.session.clone();
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            let mut session = session.lock().map_err(|_| me_core::Error::Format)?;
            let vault = session
                .as_mut()
                .ok_or(me_core::Error::Validation("Vault locked."))?;
            let message = operation(vault)?;
            let collection = vault.collection("", false)?;
            Ok::<_, me_core::Error>((message, collection))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.busy = false;
                match result {
                    Ok((message, collection)) => {
                        this.collection = collection;
                        this.notice = Some(message);
                        this.editing = None;
                        this.document_open = None;
                        this.show_info = false;
                        this.history.clear();
                        this.title_input.update(cx, |i, cx| i.set_text("", cx));
                        this.value_input.update(cx, |i, cx| i.set_text("", cx));
                        this.pinned_only = false;
                        this.search.update(cx, |i, cx| i.set_text("", cx));
                    }
                    Err(err) => {
                        this.error = Some(err.to_string());
                        this.notice = Some(err.to_string());
                    }
                }
                this.refresh_search(cx);
                this.refresh_imports(cx);
                this.kick_auto_queue(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn refresh_search(&mut self, cx: &mut Context<Self>) {
        if !self.app_ready() || self.busy {
            return;
        }
        self.search_generation += 1;
        let request = self.search_generation;
        let generation = self.generation;
        let session = self.session.clone();
        let task = cx.background_executor().spawn(async move {
            let guard = session.lock().map_err(|_| me_core::Error::Format)?;
            let vault = guard.as_ref().ok_or(me_core::Error::Format)?;
            Ok::<_, me_core::Error>((
                vault.collection("", false)?,
                vault.pending_reviews()?,
                vault.document_folders()?,
                vault.data_facts()?,
            ))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation || this.search_generation != request || this.busy {
                    return;
                }
                match result {
                    Ok((collection, reviews, folders, facts)) => {
                        this.collection = collection;
                        this.reviews = reviews;
                        this.folders = folders;
                        this.selected_reviews
                            .retain(|id| this.reviews.iter().any(|r| &r.id == id));
                        this.filter.facts = facts;
                        this.refresh_local_search(cx);
                        if this.page == Page::Knowledge {
                            this.refresh_knowledge(cx);
                        }
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn load_history(&self, item: u64, cx: &mut Context<Self>) {
        let session = self.session.clone();
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            session
                .lock()
                .map_err(|_| me_core::Error::Format)?
                .as_ref()
                .ok_or(me_core::Error::Format)?
                .history(item)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation && this.editing == Some(Some(item)) {
                    if let Ok(history) = result {
                        this.history = history;
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn clear_owned_clipboard(&mut self, cx: &mut Context<Self>) {
        self.clipboard_generation += 1;
        if let Some(value) = self.copied_value.take()
            && cx.read_from_clipboard().and_then(|v| v.text()).as_deref() == Some(value.as_str())
        {
            cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
        }
        self.copied = None;
        self.knowledge.copied = None;
    }
    pub(super) fn clear_clipboard_later(&mut self, cx: &mut Context<Self>) {
        self.clipboard_generation += 1;
        let copied = self.clipboard_generation;
        let generation = self.generation;
        let timer = cx.background_executor().timer(Duration::from_secs(30));
        cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation && this.clipboard_generation == copied {
                    this.clear_owned_clipboard(cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn pick_backup(&mut self, cx: &mut Context<Self>) {
        if self.busy || !self.app_ready() {
            return;
        }
        let generation = self.generation;
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose backup location".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = prompt.await
                && let Some(parent) = paths.first()
            {
                let path = parent.join(format!("ME-{}.mebackup", uuid::Uuid::new_v4()));
                let _ = this.update(cx, |this, cx| {
                    if this.generation == generation {
                        this.run_change(cx, move |v| {
                            v.backup(&path)?;
                            Ok("Backup created. Restore it with your vault password.".into())
                        });
                    }
                });
            }
        })
        .detach();
    }
    fn pick_restore(&mut self, cx: &mut Context<Self>) {
        if self.busy || self.initialized {
            return;
        }
        let generation = self.generation;
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose a ME. backup".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = prompt.await
                && let Some(path) = paths.first()
            {
                let path = path.clone();
                let _ = this.update(cx, |this, cx| {
                    if this.generation == generation {
                        this.restore_from = Some(path);
                        this.error = None;
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }
    fn pick_export(&mut self, item: u64, cx: &mut Context<Self>) {
        if self.busy || !self.app_ready() {
            return;
        }
        let Some(entry) = self.collection.get(item) else {
            return;
        };
        let generation = self.generation;
        let directory = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        let prompt = cx.prompt_for_new_path(&directory, Some(&entry.title));
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(path))) = prompt.await {
                let _ = this.update(cx, |this, cx| {
                    if this.generation == generation {
                        this.run_change(cx, move |v| {
                            v.export_document(item, &path)?;
                            Ok("Exported an unencrypted copy.".into())
                        });
                    }
                });
            }
        })
        .detach();
    }

    pub(super) fn locked_view(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let restoring = self.restore_from.is_some();
        let title = if self.initialized {
            "It's about you."
        } else if restoring {
            "Restore your vault."
        } else {
            "A space for you."
        };
        div()
            .id("locked-shell")
            .size_full()
            .bg(rgb(BG))
            .font_family(font::SANS)
            .type_style(Type::Body)
            .text_color(rgb(INK))
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .px(px(space::XXL))
            .key_context("Me")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::next_field))
            .on_action(cx.listener(Self::previous_field))
            .child(
                div()
                    .w(px(360.))
                    .max_w_full()
                    .mx_auto()
                    .my_auto()
                    .py(px(space::SECTION))
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .gap(px(space::XXL))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(space::LG))
                            .child(fingerprint_intro())
                            .child(div().type_style(Type::Brand).child("ME."))
                            .child(
                                div()
                                    .text_center()
                                    .flex()
                                    .flex_col()
                                    .gap(px(space::SM))
                                    .child(heading(title))
                                    .child(
                                        div()
                                            .type_style(Type::Body)
                                            .text_color(rgb(MUTED))
                                            .child("Your data. At your fingertips."),
                                    ),
                            ),
                    )
                    .when(!self.initialized, |s| {
                        s.child(div().type_style(Type::Small).text_color(rgb(MUTED)).child(
                            if restoring {
                                "Enter the password used for this backup."
                            } else {
                                "Choose a password with at least 10 characters."
                            },
                        ))
                    })
                    .child(self.editor_field("Vault password", self.password.clone(), window, cx))
                    .when(!self.initialized && !restoring, |s| {
                        s.child(self.editor_field(
                            "Repeat password",
                            self.password_repeat.clone(),
                            window,
                            cx,
                        ))
                    })
                    .when_some(self.error.clone(), |s, error| {
                        s.child(
                            div()
                                .type_style(Type::Small)
                                .text_color(rgb(DANGER))
                                .child(error),
                        )
                    })
                    .child(
                        primary_action()
                            .id("unlock-vault")
                            .h(px(layout::CONTROL_LARGE))
                            .hover(|s| s.bg(rgb(PRIMARY_HOVER)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                window.focus(&this.focus);
                                this.unlock_vault(cx);
                            }))
                            .child(if self.busy {
                                "Opening your vault…"
                            } else if self.initialized {
                                "Unlock"
                            } else if restoring {
                                "Restore"
                            } else {
                                "Create vault"
                            }),
                    )
                    .when(!self.initialized && !self.busy, |s| {
                        s.child(
                            div()
                                .id("restore-vault")
                                .type_style(Type::Small)
                                .text_color(rgb(ACCENT))
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if this.restore_from.is_some() {
                                        this.restore_from = None;
                                        cx.notify();
                                    } else {
                                        this.pick_restore(cx);
                                    }
                                }))
                                .child(if restoring {
                                    "Create a new vault"
                                } else {
                                    "Restore a backup"
                                }),
                        )
                    })
                    .when(!self.initialized, |s| {
                        s.child(
                            div()
                                .type_style(Type::Caption)
                                .text_color(rgb(MUTED))
                                .child("Keep your password safe. ME. can't recover it."),
                        )
                    }),
            )
    }

    pub(super) fn document_modal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let id = self.document_open.unwrap_or_default();
        let entry = self.collection.get(id);
        let title = entry.map(|i| i.title.clone()).unwrap_or_default();
        let state = entry
            .and_then(|i| {
                if let Content::Document { state, .. } = &i.content {
                    Some(state.as_str())
                } else {
                    None
                }
            })
            .unwrap_or("manual");
        let active = self.active_imports.contains_key(&id);
        let finished = matches!(state, "done" | "needs_review") && self.document_warning.is_none();
        let supports_text=entry.is_some_and(|i|matches!(&i.content,Content::Document{extension,..} if me_core::processable_document(extension)));
        self.overlay(cx).child(
            modal_panel(550.)
                .id("document-detail")
                .max_h(gpui::relative(0.88))
                .overflow_y_scroll()
                .child(eyebrow("ORIGINAL"))
                .when_some(self.document_warning.clone(), |s, warning| {
                    s.child(
                        div()
                            .p(px(space::MD))
                            .rounded(px(radius::STANDARD))
                            .bg(rgb(WARNING_SURFACE))
                            .text_color(rgb(WARNING))
                            .type_style(Type::Small)
                            .child(warning),
                    )
                })
                .child(div().type_style(Type::Title).text_ellipsis().child(title))
                .when_some(self.document_error.clone(), |s, error| {
                    s.child(
                        div()
                            .p(px(space::MD))
                            .rounded(px(radius::STANDARD))
                            .bg(rgb(DANGER_SURFACE))
                            .text_color(rgb(DANGER))
                            .flex()
                            .flex_col()
                            .gap(px(space::SM))
                            .child(
                                div()
                                    .type_style(Type::Body)
                                    .font_weight(font::EMPHASIS)
                                    .child("Analysis couldn't finish"),
                            )
                            .child(div().type_style(Type::Small).child(error)),
                    )
                })
                .when_some(
                    if (self.active_imports.contains_key(&id) || self.ai_item == Some(id))
                        && self.document_error.is_none()
                        && self.document_warning.is_none()
                    {
                        self.active_imports
                            .get(&id)
                            .map(|a| a.message.clone())
                            .or_else(|| self.ai_message.clone())
                    } else {
                        None
                    },
                    |s, m| {
                        s.child(
                            div()
                                .p(px(space::MD))
                                .rounded(px(radius::STANDARD))
                                .bg(rgb(BG))
                                .font_family(font::SANS)
                                .type_style(Type::Body)
                                .text_color(rgb(ACCENT))
                                .child(m),
                        )
                    },
                )
                .child(
                    div()
                        .type_style(Type::Small)
                        .text_color(rgb(MUTED))
                        .child("Export creates an unencrypted copy of your original."),
                )
                .child(
                    primary_action()
                        .id("export-document")
                        .flex_shrink_0()
                        .hover(|s| s.bg(rgb(PRIMARY_HOVER)))
                        .on_click(cx.listener(move |this, _, _, cx| this.pick_export(id, cx)))
                        .child("Export original…"),
                )
                .when(!self.questions.is_empty(), |s| {
                    s.child(
                        div()
                            .type_style(Type::Label)
                            .font_weight(font::EMPHASIS)
                            .child("Needs confirmation"),
                    )
                })
                .children(
                    self.questions
                        .iter()
                        .map(|question| self.question_card(question, cx)),
                )
                .when(supports_text && !finished && !active, |s| {
                    s.child(
                        div()
                            .type_style(Type::Small)
                            .text_color(rgb(MUTED))
                            .child("Release content for search if it contains no credentials."),
                    )
                    .child(
                        div()
                            .id("index-document")
                            .type_style(Type::Small)
                            .text_color(rgb(ACCENT))
                            .cursor_pointer()
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.run_document(id, false, cx)),
                            )
                            .child("Enable content search"),
                    )
                })
                .when(!supports_text, |s| {
                    s.child(div().type_style(Type::Small).text_color(rgb(MUTED)).child(
                        "File saved. For analysis, import a PDF, DOCX, ODT, RTF, or text version.",
                    ))
                })
                .when(supports_text, |s| {
                    s.child(div().type_style(Type::Small).text_color(rgb(MUTED)).child(
                        "Analysis sends extracted content to TypeSafe for decisions and OpenAI through your ChatGPT account for extraction.",
                    ))
                    .child(
                        div()
                            .id("extract-document")
                            .type_style(Type::Body)
                            .text_color(rgb(ACCENT))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| this.run_ai(id, cx)))
                            .child(if active {
                                "Reading…"
                            } else if self.active_imports.len() >= import_ui::MAX_ACTIVE_IMPORTS {
                                "Another file is being read"
                            } else if finished {
                                "Analyze again"
                            } else if state == "failed"
                                || state == "waiting_provider"
                                || self.document_warning.is_some()
                            {
                                "Retry analysis"
                            } else {
                                "Analyze with AI"
                            }),
                    )
                })
                .when(finished, |s| {
                    s.child(div().type_style(Type::Small).text_color(rgb(ACCENT)).child(
                        if state == "needs_review" {
                            "Ready for review."
                        } else {
                            "Analysis complete."
                        },
                    ))
                })
                .when(active, |s| {
                    s.child(
                        div()
                            .id("cancel-ai")
                            .type_style(Type::Small)
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(active) = this.active_imports.get_mut(&id) {
                                    active
                                        .cancel
                                        .store(true, std::sync::atomic::Ordering::SeqCst);
                                    active.message = "Stopping…".into();
                                }
                                this.ai_message = Some("Stopping…".into());
                                cx.notify();
                            }))
                            .child("Stop analysis"),
                    )
                })
                .when(!self.proposals.is_empty(), |s| {
                    s.child(
                        div()
                            .type_style(Type::Label)
                            .font_weight(font::EMPHASIS)
                            .child("Found in this document"),
                    )
                })
                .children(self.proposals.iter().map(|p| {
                    div()
                        .p(px(space::MD))
                        .bg(rgb(BG))
                        .rounded(px(radius::STANDARD))
                        .flex()
                        .flex_col()
                        .gap(px(space::SM))
                        .child(
                            div()
                                .type_style(Type::Small)
                                .font_weight(font::EMPHASIS)
                                .child(format!("{}: {}", p.label, p.value)),
                        )
                        .when(!p.existing_values.is_empty(), |s| {
                            s.child(
                                div()
                                    .type_style(Type::Caption)
                                    .text_color(rgb(ACCENT))
                                    .child(format!(
                                        "Already saved: {}",
                                        p.existing_values.join(" · ")
                                    )),
                            )
                        })
                        .child(
                            div()
                                .type_style(Type::Caption)
                                .text_color(rgb(MUTED))
                                .child(format!("{}: {}", p.location_label, p.quote)),
                        )
                        .child(
                            div()
                                .type_style(Type::Caption)
                                .text_color(rgb(MUTED))
                                .child(format!("Person in document: {}", p.subject_quote)),
                        )
                }))
                .when(!self.proposals.is_empty(), |s| {
                    s.child(
                        div()
                            .id("accept-proposals")
                            .type_style(Type::Body)
                            .text_color(rgb(ACCENT))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.run_change(cx, move |v| {
                                    v.review_proposals(id, true)?;
                                    Ok("Approved. Conflicting values are kept for review.".into())
                                })
                            }))
                            .child("Approve these details as mine"),
                    )
                    .child(
                        div()
                            .id("reject-proposals")
                            .type_style(Type::Small)
                            .text_color(rgb(MUTED))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.run_change(cx, move |v| {
                                    v.review_proposals(id, false)?;
                                    Ok("Suggestions dismissed.".into())
                                })
                            }))
                            .child("Dismiss suggestions"),
                    )
                })
                .when_some(self.error.clone(), |s, error| {
                    s.child(
                        div()
                            .type_style(Type::Small)
                            .text_color(rgb(DANGER))
                            .child(error),
                    )
                })
                .child(
                    div()
                        .id("close-document")
                        .type_style(Type::Small)
                        .cursor_pointer()
                        .on_click(
                            cx.listener(|this, _, window, cx| this.dismiss(&Dismiss, window, cx)),
                        )
                        .child("Close"),
                ),
        )
    }
}
