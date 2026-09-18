use super::*;
use me_agent::codex::{Progress, SetupIssue};
use std::sync::atomic::{AtomicBool, Ordering};

impl MeApp {
    pub(super) fn app_ready(&self) -> bool {
        self.codex_ready && self.unlocked
    }

    pub(super) fn schedule_codex_check(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |this, cx| this.start_codex_setup(false, cx));
        })
        .detach();
    }

    pub(super) fn stop_codex_setup(&mut self) {
        self.codex_generation += 1;
        if let Some(cancel) = self.codex_cancel.take() {
            cancel.store(true, Ordering::SeqCst);
        }
        self.codex_login_url = None;
    }

    pub(super) fn require_codex_setup(&mut self, issue: SetupIssue, cx: &mut Context<Self>) {
        self.codex_ready = false;
        self.codex_message = issue.message();
        self.codex_login_url = None;
        self.stop_ai();
        self.stop_bridge(cx);
        cx.notify();
    }

    pub(super) fn start_codex_setup(&mut self, login: bool, cx: &mut Context<Self>) {
        if self.codex_cancel.is_some() {
            return;
        }
        me_diagnostics::record(
            "codex.setup.started",
            &[me_diagnostics::Field::Flag("login", login)],
        );
        self.codex_ready = false;
        self.stop_bridge(cx);
        let Some(home) = self
            .root
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.join("codex-inbox"))
        else {
            self.codex_message = "Couldn't locate the ME. folder.".into();
            cx.notify();
            return;
        };
        self.codex_generation += 1;
        let generation = self.codex_generation;
        let cancel = Arc::new(AtomicBool::new(false));
        self.codex_cancel = Some(cancel.clone());
        self.codex_login_url = None;
        self.codex_message = if login {
            "Preparing sign-in…"
        } else {
            "Checking connection…"
        }
        .into();
        let (tx, rx) = std::sync::mpsc::channel();
        let task = cx.background_executor().spawn(async move {
            let progress = |event| {
                let _ = tx.send(event);
            };
            if login {
                me_agent::codex::configure(&home, cancel, progress)
            } else {
                me_agent::codex::check_configuration(&home, cancel, progress)
            }
        });
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            loop {
                let mut disconnected = false;
                loop {
                    let event = match rx.try_recv() {
                        Ok(event) => event,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            disconnected = true;
                            break;
                        }
                        Err(_) => break,
                    };
                    let _ = this.update(cx, |this, cx| {
                        if this.codex_generation != generation
                            || this
                                .codex_cancel
                                .as_ref()
                                .is_none_or(|c| c.load(Ordering::SeqCst))
                        {
                            return;
                        }
                        match event {
                            Progress::Stage(_) | Progress::Units { .. } => {}
                            Progress::Message(message) => this.codex_message = message,
                            Progress::LoginUrl(url) => {
                                cx.open_url(&url);
                                this.codex_login_url = Some(url);
                                this.codex_message =
                                    "Sign in to ChatGPT in your browser to continue.".into();
                            }
                            Progress::SetupRequired(issue) => this.codex_message = issue.message(),
                        }
                        cx.notify();
                    });
                }
                if disconnected {
                    break;
                }
                executor.timer(std::time::Duration::from_millis(100)).await;
            }
            let result = task.await;
            me_diagnostics::record(
                "codex.setup.finished",
                &[me_diagnostics::Field::Flag("ok", result.is_ok())],
            );
            let _ = this.update(cx, |this, cx| {
                if this.codex_generation != generation {
                    return;
                }
                let cancelled = this
                    .codex_cancel
                    .take()
                    .is_none_or(|c| c.load(Ordering::SeqCst));
                this.codex_login_url = None;
                if cancelled {
                    this.codex_ready = false;
                    this.codex_message = "Setup stopped. Connect ChatGPT to continue.".into();
                } else {
                    match result {
                        Ok(()) => {
                            this.codex_ready = true;
                            this.codex_message.clear();
                            this.refresh_search(cx);
                            this.kick_auto_queue(cx);
                        }
                        Err(issue) => {
                            this.codex_ready = false;
                            this.codex_message = issue.message();
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn codex_setup_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let working = self.codex_cancel.is_some();
        div()
            .id("codex-setup-shell")
            .size_full()
            .bg(rgb(BG))
            .font_family(font::SANS)
            .type_style(Type::Body)
            .text_color(rgb(INK))
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .key_context("Me")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::lock_vault))
            .child(
                div()
                    .w(px(460.))
                    .mx_auto()
                    .my_auto()
                    .py(px(space::SECTION))
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .gap(px(space::XXL))
                    .child(
                        div()
                            .type_style(Type::Brand)
                            .font_weight(font::EMPHASIS)
                            .child("ME."),
                    )
                    .child(eyebrow("SETUP"))
                    .child(heading("Connect ChatGPT."))
                    .child(div().type_style(Type::Body).text_color(rgb(MUTED)).child(
                        "ME. uses your ChatGPT subscription to read files and find your data.",
                    ))
                    .child(
                        div()
                            .p(px(space::LG))
                            .rounded(px(radius::STANDARD))
                            .bg(rgb(SURFACE))
                            .type_style(Type::Small)
                            .child(self.codex_message.clone()),
                    )
                    .child(
                        primary_action()
                            .id("configure-codex")
                            .bg(rgb(if working { MUTED } else { INK }))
                            .h(px(layout::CONTROL_LARGE))
                            .on_click(
                                cx.listener(|this, _, _, cx| this.start_codex_setup(true, cx)),
                            )
                            .child(if working {
                                "One moment…"
                            } else {
                                "Connect ChatGPT"
                            }),
                    )
                    .when_some(self.codex_login_url.clone(), |s, url| {
                        s.child(
                            div()
                                .id("reopen-codex-login")
                                .type_style(Type::Small)
                                .text_color(rgb(ACCENT))
                                .cursor_pointer()
                                .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url)))
                                .child("Open sign-in page"),
                        )
                    })
                    .when(!working, |s| {
                        s.child(
                            div()
                                .id("check-codex")
                                .type_style(Type::Small)
                                .text_color(rgb(ACCENT))
                                .cursor_pointer()
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.start_codex_setup(false, cx)),
                                )
                                .child("Check connection"),
                        )
                    })
                    .when(working, |s| {
                        s.child(
                            div()
                                .id("cancel-codex-setup")
                                .type_style(Type::Small)
                                .text_color(rgb(MUTED))
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(cancel) = &this.codex_cancel {
                                        cancel.store(true, Ordering::SeqCst);
                                    }
                                    this.codex_login_url = None;
                                    this.codex_message = "Stopping…".into();
                                    cx.notify();
                                }))
                                .child("Cancel"),
                        )
                    })
                    .child(
                        div()
                            .id("codex-install-help")
                            .type_style(Type::Small)
                            .text_color(rgb(MUTED))
                            .cursor_pointer()
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.open_url("https://learn.chatgpt.com/docs/cli")
                            }))
                            .child("Install or update Codex"),
                    )
                    .child(
                        div()
                            .id("setup-copy-diagnostics")
                            .type_style(Type::Small)
                            .text_color(rgb(MUTED))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    me_diagnostics::report(),
                                ));
                                this.codex_message = "Diagnostics copied.".into();
                                cx.notify();
                            }))
                            .child("Copy diagnostics"),
                    )
                    .when(self.unlocked, |s| {
                        s.child(
                            div()
                                .id("lock-during-codex-setup")
                                .type_style(Type::Small)
                                .text_color(rgb(MUTED))
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.lock_vault(&LockVault, window, cx)
                                }))
                                .child("Lock ME."),
                        )
                    })
                    .child(
                        div()
                            .type_style(Type::Caption)
                            .text_color(rgb(MUTED))
                            .child("Automatic file analysis can be changed in Settings."),
                    ),
            )
    }
}
