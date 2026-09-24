use super::*;
use me_agent::codex::{Progress, SetupIssue};
use std::sync::atomic::{AtomicBool, Ordering};

impl MeApp {
    pub(super) fn app_ready(&self) -> bool {
        self.unlocked && !self.provider_setup_visible()
    }

    pub(super) fn ai_ready(&self) -> bool {
        self.app_ready() && self.codex_ready
    }

    pub(super) fn stop_codex_setup(&mut self) {
        self.codex_generation += 1;
        if let Some(cancel) = self.codex_cancel.take() {
            cancel.store(true, Ordering::SeqCst);
        }
        self.codex_login_url = None;
        self.codex_login_requested = false;
    }

    pub(super) fn require_codex_setup(&mut self, issue: SetupIssue, cx: &mut Context<Self>) {
        self.stop_codex_setup();
        self.codex_ready = false;
        self.codex_message = issue.message();
        self.codex_issue = Some(issue);
        self.codex_notice = true;
        self.stop_filter();
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
        self.codex_issue = None;
        self.codex_notice = false;
        self.stop_bridge(cx);
        let Some(home) = self
            .root
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.join("codex-inbox"))
        else {
            self.require_codex_setup(
                SetupIssue::Connection("Couldn't locate the ME. folder.".into()),
                cx,
            );
            return;
        };
        self.codex_generation += 1;
        let generation = self.codex_generation;
        let cancel = Arc::new(AtomicBool::new(false));
        self.codex_cancel = Some(cancel.clone());
        self.codex_login_requested = login;
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
                            Progress::Stage(_)
                            | Progress::Units { .. }
                            | Progress::Step { .. }
                            | Progress::Failure(_)
                            | Progress::Request { .. }
                            | Progress::Usage { .. } => {}
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
                this.codex_login_requested = false;
                if cancelled {
                    this.codex_ready = false;
                    this.codex_message = "Connection check stopped. You can try again.".into();
                    this.codex_issue = Some(SetupIssue::Connection(this.codex_message.clone()));
                    this.codex_notice = true;
                } else {
                    match result {
                        Ok(()) => {
                            this.codex_ready = true;
                            this.codex_message.clear();
                            this.codex_issue = None;
                            this.codex_notice = false;
                            this.show_codex_setup = false;
                            this.refresh_search(cx);
                            this.kick_auto_queue(cx);
                        }
                        Err(issue) => {
                            this.require_codex_setup(issue, cx);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn codex_issue_title(&self) -> &'static str {
        match self.codex_issue {
            Some(SetupIssue::UsageLimit) => "ChatGPT usage limit reached",
            Some(SetupIssue::SignInRequired | SetupIssue::WrongAccount) => "Connect ChatGPT",
            _ => "ChatGPT is unavailable",
        }
    }

    pub(super) fn codex_notice_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div().mx(px(space::PAGE)).mt(px(space::TITLEBAR)).p(px(space::LG))
            .rounded(px(radius::STANDARD)).bg(rgb(WARNING_SURFACE))
            .flex().items_center().gap(px(space::LG)).flex_shrink_0()
            .child(div().flex_1().min_w_0().flex().flex_col().gap(px(space::XS))
                .child(div().type_style(Type::Small).text_color(rgb(WARNING)).child(self.codex_issue_title()))
                .child(div().type_style(Type::Small).text_color(rgb(WARNING))
                    .child("Your vault is open. AI features will be available when ChatGPT is ready.")))
            .child(div().id("codex-notice-details").type_style(Type::Small).text_color(rgb(ACCENT))
                .cursor_pointer().on_click(cx.listener(|this, _, _, cx| {
                    this.show_codex_setup = true; cx.notify();
                })).child("Details"))
            .child(div().id("dismiss-codex-notice").type_style(Type::Small).text_color(rgb(MUTED))
                .cursor_pointer().on_click(cx.listener(|this, _, _, cx| {
                    this.codex_notice = false; cx.notify();
                })).child("Dismiss"))
    }

    pub(super) fn codex_setup_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let working = self.codex_cancel.is_some();
        let sign_in = matches!(
            self.codex_issue,
            Some(SetupIssue::SignInRequired | SetupIssue::WrongAccount)
        );
        self.overlay(cx).child(
            modal_panel(486., self.motion_enabled())
                .child(heading(if working {
                    "Connecting to ChatGPT"
                } else {
                    self.codex_issue_title()
                }))
                .child(div().type_style(Type::Body).text_color(rgb(MUTED)).child(
                    "You can keep using your vault. ChatGPT powers AI search and file analysis.",
                ))
                .child(
                    div()
                        .p(px(space::LG))
                        .rounded(px(radius::STANDARD))
                        .bg(rgb(BG))
                        .type_style(Type::Small)
                        .child(self.codex_message.clone()),
                )
                .when(!working, |s| {
                    s.child(
                        primary_action()
                            .id("configure-codex")
                            .hover(|s| s.bg(rgb(PRIMARY_HOVER)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.start_codex_setup(sign_in, cx)
                            }))
                            .child(if sign_in {
                                "Connect ChatGPT"
                            } else {
                                "Check again"
                            }),
                    )
                })
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
                .when(
                    !working
                        && !sign_in
                        && !matches!(self.codex_issue, Some(SetupIssue::UsageLimit)),
                    |s| {
                        s.child(
                            div()
                                .id("codex-sign-in-again")
                                .type_style(Type::Small)
                                .text_color(rgb(ACCENT))
                                .cursor_pointer()
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.start_codex_setup(true, cx)),
                                )
                                .child("Sign in again"),
                        )
                    },
                )
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
                            .child("Cancel connection check"),
                    )
                })
                .when(
                    matches!(
                        self.codex_issue,
                        Some(SetupIssue::Connection(_) | SetupIssue::ModelUnavailable)
                    ),
                    |s| {
                        s.child(
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
                    },
                )
                .child(
                    div()
                        .id("close-codex-setup")
                        .type_style(Type::Small)
                        .text_color(rgb(ACCENT))
                        .cursor_pointer()
                        .on_click(
                            cx.listener(|this, _, window, cx| this.dismiss(&Dismiss, window, cx)),
                        )
                        .child("Back to workspace"),
                ),
        )
    }
}
