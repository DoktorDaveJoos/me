use super::*;
use me_agent::codex::SetupIssue;

impl MeApp {
    pub(super) fn provider_setup_visible(&self) -> bool {
        self.unlocked && self.account.binding.is_some() && !self.settings.onboarding_complete
    }

    /// Completing the introduction never bypasses the live provider check. The
    /// saved flag only prevents later offline launches from repeating onboarding.
    pub(super) fn finish_onboarding(&mut self, cx: &mut Context<Self>) {
        if !self.provider_setup_visible() || !self.codex_ready || self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let session = self.session.clone();
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            let mut guard = session.lock().map_err(|_| me_core::Error::Format)?;
            let vault = guard.as_mut().ok_or(me_core::Error::Format)?;
            vault.complete_onboarding()?;
            vault.settings()
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.generation {
                    return;
                }
                this.busy = false;
                match result {
                    Ok(settings) => {
                        this.settings = settings;
                        this.focus_filter_on_ready = true;
                        this.refresh_search(cx);
                        this.refresh_imports(cx);
                        this.kick_auto_queue(cx);
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn onboarding_shell(
        &self,
        step: usize,
        title: &'static str,
        subtitle: &'static str,
        body: gpui::AnyElement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let wide = window.viewport_size().width >= px(layout::ONBOARDING_WIDTH);
        let fingerprint_phase =
            self.window_entrance_phase(window, crate::design_system::motion::FINGERPRINT_ENTER_MS);
        let unlocking = !self.unlocked && self.initialized && !self.account_setup_visible();
        let labels: &[&str] = if unlocking {
            &["Unlock"]
        } else {
            match self.account.mode {
                AccountMode::SignIn => &["Sign in"],
                AccountMode::Recover => &["Recovery details", "New recovery code"],
                _ => &["Account", "Recovery", "ChatGPT"],
            }
        };
        let rail =
            div()
                .flex()
                .gap(px(space::SM))
                .children(labels.iter().copied().enumerate().map(|(index, label)| {
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(space::SM))
                        .child(div().h(px(space::MICRO)).bg(rgb(if index <= step {
                            ACCENT
                        } else {
                            LINE
                        })))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(space::XS))
                                .type_style(Type::Caption)
                                .text_color(rgb(if index == step { ACCENT } else { MUTED }))
                                .child(if index < step {
                                    icon(Icon::Check, IconSize::Small, ACCENT).into_any_element()
                                } else {
                                    div()
                                        .font_family(font::MONO)
                                        .child(format!("0{}", index + 1))
                                        .into_any_element()
                                })
                                .child(label),
                        )
                }));
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
                    .child(body),
            );
        div().id(("account-shell", step + self.account.mode as usize * 3)).size_full().overflow_y_scroll().px(px(space::XXL))
            .flex().flex_col().key_context("Me").track_focus(&self.focus)
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::next_field))
            .on_action(cx.listener(Self::previous_field))
            .child(div().w(px(layout::ONBOARDING_WIDTH)).max_w_full().mx_auto().my_auto()
                .pt(px(space::TITLEBAR)).pb(px(space::PAGE_TOP)).flex_shrink_0()
                .flex().items_center().justify_center().gap(px(space::PAGE))
                .when(wide, |s| s.child(div().flex_1().min_w_0().flex().flex_col().gap(px(space::SECTION))
                    .child(brand_signature(false, window))
                    .child(div().relative().w_full().h(px(layout::ONBOARDING_ART_SIZE))
                        .when(self.motion.loaded, |s| s.child(div().absolute()
                            .right(px(-space::PAGE - layout::FINGERPRINT_ART_SIZE / 2.))
                            .top(px((layout::ONBOARDING_ART_SIZE - layout::FINGERPRINT_ART_SIZE) / 2.))
                            .child(onboarding_fingerprint(fingerprint_phase)))))
                    .child(div().flex().flex_col().gap(px(space::MD))
                        .child(heading(if unlocking { "Your life. Brought together." } else { match step { 0 => "Your life. Brought together.", 1 => "Your way back in.", _ => "Give your space a spark." }}))
                        .child(div().type_style(Type::Body).text_color(rgb(MUTED)).child(if unlocking {
                            "Your notes, documents and everyday details, all within reach. Unlock ME. and pick up where life left off."
                        } else { match step {
                            0 => "Notes, documents and the details that matter. Give them to ME. to keep them organized, connected and close at hand.",
                            1 => "A recovery code keeps you in control, even if you forget your master password.",
                            _ => "Connect ChatGPT to bring intelligent search and document understanding to your vault.",
                        }})))
                    .child(div().flex().items_center().gap(px(space::SM)).text_color(rgb(MUTED)).type_style(Type::Caption)
                        .child(icon(if step == 2 { Icon::Spark } else { Icon::Fingerprint }, IconSize::Medium, ACCENT))
                        .child(if step == 2 { "Powered by your ChatGPT subscription." } else { "Encrypted on your device." }))))
                .child(div().w(px(layout::ONBOARDING_PANEL_WIDTH)).max_w_full().flex().flex_col().gap(px(space::XL))
                    .when(!wide, |s| s.child(brand_signature(true, window)))
                    .child(motion::page(panel.into_any_element(), step + self.account.mode as usize * 3, self.motion_enabled()))))
            .into_any_element()
    }

    pub(super) fn provider_onboarding_view(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let working = self.codex_cancel.is_some();
        let ready = self.codex_ready;
        let sign_in = matches!(
            self.codex_issue,
            None | Some(SetupIssue::SignInRequired | SetupIssue::WrongAccount)
        );
        let detail = if ready {
            "Your ChatGPT account is connected and ready for ME.".to_owned()
        } else if matches!(self.codex_issue, Some(SetupIssue::UsageLimit)) {
            "Your ChatGPT account is signed in, but its usage allowance is exhausted. Check again when it resets to finish setup.".into()
        } else if self.codex_message.is_empty() {
            "Sign in with your ChatGPT account. We’ll check that it has access to ME.’s AI features.".into()
        } else {
            self.codex_message.clone()
        };
        let body = div().flex().flex_col().gap(px(space::LG))
            .child(div().p(px(space::LG)).rounded(px(radius::STANDARD)).bg(rgb(BG)).border_1().border_color(rgb(LINE))
                .flex().flex_col().gap(px(space::MD))
                .child(div().flex().items_center().gap(px(space::MD))
                    .child(icon(Icon::Spark, IconSize::Large, ACCENT))
                    .child(div().flex_1().flex().flex_col()
                        .child(div().type_style(Type::Label).child("ChatGPT"))
                        .child(eyebrow("OPENAI")))
                    .child(div().flex().items_center().gap(px(space::XS)).type_style(Type::Caption)
                        .text_color(rgb(if ready { SUCCESS } else { MUTED }))
                        .when(ready, |s| s.child(icon(Icon::Check, IconSize::Medium, SUCCESS)))
                        .child(if ready { "Connected" } else if working { "Connecting…" } else { "Required" })))
                .child(div().type_style(Type::Small).text_color(rgb(MUTED)).child(detail)))
            .child(div().type_style(Type::Small).text_color(rgb(MUTED))
                .child("Continue securely in your browser using your existing ChatGPT subscription. It can use a different email from your ME. account."))
            .when_some(self.error.clone(), |s, error| s.child(div().p(px(space::MD)).rounded(px(radius::STANDARD))
                .bg(rgb(DANGER_SURFACE)).text_color(rgb(DANGER)).type_style(Type::Small).child(error)))
            .when(ready, |s| s.child(primary_action().id("finish-onboarding").h(px(layout::CONTROL_LARGE))
                .hover(|s| s.bg(rgb(PRIMARY_HOVER))).when(self.busy, |s| s.cursor_default())
                .on_click(cx.listener(|this, _, _, cx| this.finish_onboarding(cx)))
                .child(if self.busy { "Finishing setup…" } else { "Enter ME." })
                .child(action_indicator(self.busy, self.motion_enabled()))))
            .when(!ready && !working, |s| s.child(primary_action().id("onboarding-connect-chatgpt").h(px(layout::CONTROL_LARGE))
                .hover(|s| s.bg(rgb(PRIMARY_HOVER)))
                .on_click(cx.listener(move |this, _, _, cx| this.start_codex_setup(sign_in, cx)))
                .child(if sign_in { "Connect ChatGPT" } else { "Check connection again" })
                .child(icon(Icon::Arrow, IconSize::Medium, SURFACE))))
            .when(working, |s| s.child(secondary_action().id("onboarding-cancel-connection")
                .hover(|s| s.bg(rgb(HOVER)))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.stop_codex_setup();
                    this.codex_ready = false;
                    this.codex_issue = Some(SetupIssue::SignInRequired);
                    this.codex_message = "Sign-in stopped. Connect ChatGPT when you’re ready.".into();
                    cx.notify();
                })).child("Cancel connection")))
            .when_some(self.codex_login_url.clone(), |s, url| s.child(secondary_action().id("onboarding-open-signin")
                .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url))).child("Open sign-in page")))
            .when(!ready && !working && !sign_in, |s| s.child(secondary_action().id("onboarding-signin-again")
                .on_click(cx.listener(|this, _, _, cx| this.start_codex_setup(true, cx))).child("Sign in with ChatGPT")))
            .when(matches!(self.codex_issue, Some(SetupIssue::Connection(_) | SetupIssue::ModelUnavailable)), |s| s
                .child(secondary_action().id("onboarding-install-help").on_click(cx.listener(|_, _, _, cx| {
                    cx.open_url("https://learn.chatgpt.com/docs/cli")
                })).child("Install or update Codex")))
            .child(secondary_action().id("onboarding-logout").hover(|s| s.bg(rgb(HOVER)))
                .on_click(cx.listener(|this, _, _, cx| this.log_out(cx))).child("Log out"))
            .child(div().pt(px(space::SM)).border_t_1().border_color(rgb(LINE)).flex().items_start().gap(px(space::SM))
                .child(icon(Icon::Check, IconSize::Medium, SUCCESS))
                .child(div().flex_1().min_w_0().type_style(Type::Caption).text_color(rgb(MUTED))
                    .child("Your ME. account is saved. You can close the app and finish this step after unlocking again.")));
        self.onboarding_shell(
            2,
            if ready {
                "You’re all connected."
            } else {
                "Connect ChatGPT."
            },
            "One last step. Bring AI search and document understanding to your vault.",
            body.into_any_element(),
            window,
            cx,
        )
    }
}
