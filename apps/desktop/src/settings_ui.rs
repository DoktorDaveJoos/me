use super::*;
use std::sync::atomic::Ordering;

impl MeApp {
    pub(super) fn open_settings(
        &mut self,
        _: &OpenSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.app_ready() || self.busy || self.login_edit_guard(cx) {
            return;
        }
        self.dismiss(&Dismiss, window, cx);
        self.show_settings = true;
        self.show_onepassword = false;
        window.focus(&self.focus);
        cx.notify();
    }

    fn toggle_automatic_evaluation(&mut self, cx: &mut Context<Self>) {
        if !self.app_ready() || self.settings_saving {
            return;
        }
        let enabled = !self.settings.automatic_evaluation;
        me_diagnostics::record(
            "settings.automatic_evaluation",
            &[me_diagnostics::Field::Flag("enabled", enabled)],
        );
        let previous = self.settings;
        self.settings.automatic_evaluation = enabled;
        self.settings_saving = true;
        self.error = None;
        if !enabled {
            for active in self.active_imports.values().filter(|a| a.automatic) {
                active.cancel.store(true, Ordering::SeqCst);
            }
        }
        let session = self.session.clone();
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            session
                .lock()
                .map_err(|_| me_core::Error::Format)?
                .as_mut()
                .ok_or(me_core::Error::Validation("Vault locked."))?
                .set_automatic_evaluation(enabled)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.settings_saving = false;
                match result {
                    Ok(()) => {
                        this.notice = Some(
                            if enabled {
                                "Automatic reading is on."
                            } else {
                                "Automatic reading is off."
                            }
                            .into(),
                        )
                    }
                    Err(error) => {
                        this.settings = previous;
                        this.error = Some(error.to_string());
                    }
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

    pub(super) fn settings_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let automatic = self.settings.automatic_evaluation;
        div().id("settings-page").flex_1().min_h_0().pt(px(space::PAGE_TOP)).font_family(font::SANS).type_style(Type::Body).text_color(rgb(INK))
            .key_context("Me").track_focus(&self.focus).flex().flex_col()
            .on_action(cx.listener(Self::dismiss)).on_action(cx.listener(Self::lock_vault))
            .child(div().id("settings-scroll").flex_1().min_h_0().overflow_y_scroll().px(px(space::PAGE)).pb(px(space::PAGE))
                .child(div().w_full().max_w(px(layout::CONTENT_WIDTH)).mx_auto().flex().flex_col().gap(px(space::XXXL))
                    .child(heading("Settings"))
                    .when(cfg!(target_os="macos"), |s| s.child(secondary_action().id("settings-privacy-permissions").on_click(cx.listener(|this,_,window,cx|this.open_permissions(&OpenPermissions,window,cx))).child("Privacy permissions…")))
                    .when_some(self.account.binding.as_ref(), |s, binding| s.child(
                        div().p(px(space::LG)).rounded(px(radius::STANDARD)).bg(rgb(SURFACE))
                            .border_1().border_color(rgb(LINE)).flex().flex_col().gap(px(space::MD))
                            .child(eyebrow("ACCOUNT"))
                            .child(div().type_style(Type::Body).child(binding.account.email.clone()))
                            .child(div().type_style(Type::Small).text_color(rgb(MUTED))
                                .child("This account is remembered on this device. Lock ME. to unlock again with just your master password."))
                            .child(div().flex().items_center().gap(px(space::SM))
                                .child(secondary_action().id("settings-logout").hover(|s| s.bg(rgb(HOVER)))
                                    .on_click(cx.listener(|this, _, _, cx| this.log_out(cx))).child("Log out"))
                                .child(secondary_action().id("settings-register").hover(|s| s.bg(rgb(HOVER)))
                                    .on_click(cx.listener(|this, _, _, cx| this.set_account_mode(AccountMode::Register, cx)))
                                    .child("Register a new account")))
                            .child(div().type_style(Type::Caption).text_color(rgb(MUTED))
                                .child("Logging out keeps this account’s encrypted vault on this device."))))
                    .child(div().id("import-passwords").type_style(Type::Body).cursor_pointer().on_click(cx.listener(|this,_,_,cx|{this.show_onepassword = !this.show_onepassword;cx.notify();})).child("Import from 1Password…"))
                    .when(self.show_onepassword,|s|s.child(self.onepassword_settings(cx)))
                    .child(div().py(px(space::LG)).border_b_1().border_color(rgb(LINE)).flex().items_start().gap(px(space::XXL))
                        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(space::SM))
                            .child(div().type_style(Type::Label).child("Read new files automatically"))
                            .child(div().w_full().whitespace_normal().type_style(Type::Small).text_color(rgb(MUTED)).child("Extracted content goes to TypeSafe for decisions and OpenAI through your ChatGPT account for extraction. Turn off to start analysis manually.")))
                        .child(div().id("automatic-evaluation-toggle").flex_shrink_0().w(px(54.)).h(px(28.)).rounded_full().bg(rgb(if automatic{ACCENT}else{LINE})).text_color(rgb(if automatic{SURFACE}else{INK})).type_style(Type::Caption).flex().items_center().justify_center().cursor_pointer()
                            .on_click(cx.listener(|this,_,_,cx|this.toggle_automatic_evaluation(cx))).child(if self.settings_saving{"…"}else if automatic{"On"}else{"Off"})))
                    .child(div().py(px(space::LG)).border_b_1().border_color(rgb(LINE)).flex().items_start().gap(px(space::XXL))
                        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(space::SM))
                            .child(div().type_style(Type::Label).child("Reduce motion"))
                            .child(div().type_style(Type::Small).text_color(rgb(MUTED)).child("Keep the honeycomb design with instant transitions. Applies to this device, including the unlock screen.")))
                        .child(secondary_action().id("reduce-motion-toggle").hover(|s|s.bg(rgb(HOVER))).on_click(cx.listener(|this,_,_,cx|this.toggle_motion(cx)))
                            .child(if self.motion.saving {"Saving…"}else if self.motion.reduced {"On"}else{"Off"})))
                    .child(div().py(px(space::LG)).border_b_1().border_color(rgb(LINE)).flex().items_center().justify_between().gap(px(space::LG))
                        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(space::SM))
                            .child(div().type_style(Type::Label).child("Website icons"))
                            .child(div().type_style(Type::Small).text_color(rgb(MUTED)).whitespace_normal().child("Load logos from your login websites. Websites receive your IP address, never your passwords.")))
                        .child(secondary_action().flex_shrink_0().id("website-icons-toggle").hover(|s|s.bg(rgb(HOVER))).on_click(cx.listener(|this,_,_,cx|this.toggle_website_icons(cx)))
                            .child(if self.motion.saving {"Saving…"}else if self.motion.website_icons {"On"}else{"Off"})))
                    .when_some(self.motion.error.clone(),|s,error|s.child(div().type_style(Type::Small).text_color(rgb(DANGER)).child(error)))
                    .child(div().flex().items_center().justify_between()
                        .child(div().type_style(Type::Body).child("Language"))
                        .child(div().type_style(Type::Body).text_color(rgb(MUTED)).child("English")))
                    .child(div().flex().items_center().justify_between()
                        .child(div().type_style(Type::Body).child(if self.codex_ready { "ChatGPT connected" } else if self.codex_cancel.is_some() { "Checking ChatGPT connection…" } else { "ChatGPT needs attention" }))
                        .child(div().id("settings-check-codex").type_style(Type::Small).text_color(rgb(ACCENT)).cursor_pointer()
                            .on_click(cx.listener(|this,_,_,cx|{if this.active_imports.is_empty() && this.filter.cancel.is_none(){this.show_codex_setup=true;this.start_codex_setup(false,cx);}})).child("Check connection")))
                    .child(div().pt(px(space::MD)).border_t_1().border_color(rgb(LINE)).flex().flex_col().gap(px(space::XXL))
                        .child(div().id("backup-vault").type_style(Type::Body).cursor_pointer().on_click(cx.listener(|this,_,_,cx|this.pick_backup(cx))).child("Back up vault…"))
                        .child(div().id("settings-lock").type_style(Type::Body).cursor_pointer().on_click(cx.listener(|this,_,window,cx|this.lock_vault(&LockVault,window,cx))).child("Lock ME."))
                        .child(div().id("settings-access").type_style(Type::Body).cursor_pointer().on_click(cx.listener(|this,_,_,cx|{this.show_settings=false;this.show_info=true;cx.notify();})).child("Connected access")))
                    .map(|s| {
                        #[cfg(any(debug_assertions, feature = "development-tools"))]
                        let s = s.child(self.development_settings(cx));
                        s
                    })
                    .child(div().pt(px(space::MD)).border_t_1().border_color(rgb(LINE)).flex().items_center().justify_between()
                        .child(div().type_style(Type::Caption).text_color(rgb(FAINT)).child(concat!("ME. ",env!("CARGO_PKG_VERSION"))))
                        .child(div().id("copy-diagnostics").type_style(Type::Caption).text_color(rgb(MUTED)).cursor_pointer().on_click(cx.listener(|this,_,_,cx|{cx.write_to_clipboard(ClipboardItem::new_string(me_diagnostics::report()));this.notice=Some("Diagnostics copied.".into());cx.notify();})).child("Copy diagnostics")))
                    .when_some(self.error.clone(),|s,e|s.child(div().w_full().whitespace_normal().type_style(Type::Small).text_color(rgb(DANGER)).child(e)))
                    .when_some(self.notice.clone(),|s,m|s.child(div().w_full().whitespace_normal().type_style(Type::Small).text_color(rgb(MUTED)).child(m)))))
    }
}
