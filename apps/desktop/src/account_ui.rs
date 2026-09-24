use super::*;
use me_core::account::{self, AccountBinding};
use me_protocol::{AccountResponse, Recovery, Registration};
use zeroize::Zeroizing;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum AccountMode {
    #[default]
    Register,
    SignIn,
    Recover,
    Unlock,
}
#[derive(Default)]
pub(super) struct AccountState {
    pub binding: Option<AccountBinding>,
    pub mode: AccountMode,
    pub pending: Option<Arc<PendingAccount>>,
    pub signed_out: bool,
    pub saved: bool,
    pub copied: bool,
    pub complete: Option<String>,
}
pub(super) struct PendingAccount {
    pub root: Option<PathBuf>,
    pub server: String,
    pub password: Zeroizing<String>,
    pub code: Zeroizing<String>,
    pub operation: PendingOperation,
}
pub(super) enum PendingOperation {
    Register(Registration),
    Recover(Recovery),
}

impl MeApp {
    pub(super) fn account_setup_visible(&self) -> bool {
        self.restore_from.is_none() && self.account.mode != AccountMode::Unlock
    }
    fn account_server(&self) -> Result<String, String> {
        let configured = account_client::server()?;
        if self
            .account
            .binding
            .as_ref()
            .is_some_and(|b| b.server != configured)
        {
            return Err("This build uses a different account service from your vault.".into());
        }
        Ok(configured)
    }
    pub(super) fn set_account_mode(&mut self, mode: AccountMode, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if mode == AccountMode::Register
            && (self.account.binding.is_some() || self.account.signed_out)
        {
            self.sign_out_to(mode, cx);
            return;
        }
        self.account.mode = mode;
        self.account.pending = None;
        self.account.saved = false;
        self.account.copied = false;
        self.account.complete = None;
        self.error = None;
        self.clear_owned_clipboard(cx);
        self.password.update(cx, |i, cx| i.set_text("", cx));
        self.password_repeat.update(cx, |i, cx| i.set_text("", cx));
        self.recovery_input.update(cx, |i, cx| i.set_text("", cx));
        self.focus_password_on_ready = true;
        cx.notify();
    }
    pub(super) fn submit_account(&mut self, cx: &mut Context<Self>) {
        if self.busy || self.account.complete.is_some() {
            return;
        }
        if self.account.pending.is_some() {
            self.commit_account(cx);
            return;
        }
        let (Some(base), Some(root)) = (Self::vault_path(), self.root.clone()) else {
            return;
        };
        let server = match self.account_server() {
            Ok(s) => s,
            Err(e) => {
                self.error = Some(e);
                cx.notify();
                return;
            }
        };
        let email = match me_protocol::normalize_email(self.account_email.read(cx).content.as_ref())
        {
            Ok(email) => email,
            Err(e) => {
                self.error = Some(e.into());
                cx.notify();
                return;
            }
        };
        if self
            .account
            .binding
            .as_ref()
            .is_some_and(|b| b.account.email != email)
        {
            self.error = Some("Use the account connected to this device.".into());
            cx.notify();
            return;
        }
        let password = Zeroizing::new(self.password.read(cx).content.to_string());
        let code = Zeroizing::new(self.recovery_input.read(cx).content.to_string());
        let mode = self.account.mode;
        if mode != AccountMode::SignIn {
            if let Err(e) = me_protocol::validate_password(&password) {
                self.error = Some(e.into());
                cx.notify();
                return;
            }
            if password.as_str() != self.password_repeat.read(cx).content.as_ref() {
                self.error = Some("Passwords don't match.".into());
                cx.notify();
                return;
            }
        }
        self.busy = true;
        self.error = None;
        let session = self.session.clone();
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            match mode {
                AccountMode::Register => {
                    let code = account::generate_recovery_code().map_err(|e| e.to_string())?;
                    let request = account::prepare_registration(&root, &email, &password, &code)
                        .map_err(|e| e.to_string())?;
                    Ok(AccountOutcome::Prepared(Arc::new(PendingAccount {
                        root: Some(root),
                        server,
                        password,
                        code,
                        operation: PendingOperation::Register(request),
                    })))
                }
                AccountMode::Recover => {
                    let response = account_client::recovery_account(&server, &email, &code)?;
                    let new_code = account::generate_recovery_code().map_err(|e| e.to_string())?;
                    let request = account::prepare_recovery(&response, &code, &password, &new_code)
                        .map_err(|e| e.to_string())?;
                    let root = device_accounts::find_local(&base, &server, &response)?;
                    Ok(AccountOutcome::Prepared(Arc::new(PendingAccount {
                        root,
                        server,
                        password,
                        code: new_code,
                        operation: PendingOperation::Recover(request),
                    })))
                }
                AccountMode::SignIn => {
                    let response = account_client::sign_in(&server, &email, &password)?;
                    let root = device_accounts::find_local(&base, &server, &response)?;
                    open_response(
                        &base,
                        root.as_deref(),
                        &password,
                        &server,
                        response,
                        session,
                    )
                }
                AccountMode::Unlock => unreachable!(),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if generation == this.generation {
                    this.finish_account(result, cx);
                }
            });
        })
        .detach();
        cx.notify();
    }
    fn commit_account(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if !self.account.saved {
            self.error = Some("Save your recovery code, then confirm below.".into());
            cx.notify();
            return;
        }
        let Some(pending) = self.account.pending.clone() else {
            return;
        };
        let Some(base) = Self::vault_path() else {
            return;
        };
        self.busy = true;
        self.error = None;
        let session = self.session.clone();
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            let response = match &pending.operation {
                PendingOperation::Register(request) => {
                    account::stage_registration(
                        pending
                            .root
                            .as_deref()
                            .ok_or("Missing local vault location.")?,
                        &pending.password,
                        request,
                    )
                    .map_err(|e| e.to_string())?;
                    account_client::register(&pending.server, request)?
                }
                PendingOperation::Recover(request) => {
                    account_client::recover(&pending.server, request)?
                }
            };
            open_response(
                &base,
                pending.root.as_deref(),
                &pending.password,
                &pending.server,
                response,
                session,
            )
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if generation == this.generation {
                    this.finish_account(result, cx);
                }
            });
        })
        .detach();
        cx.notify();
    }
    fn finish_account(&mut self, result: Result<AccountOutcome, String>, cx: &mut Context<Self>) {
        self.busy = false;
        // Preserve masked fields during work and on retry; clear only after success.
        if result.is_ok() {
            self.password.update(cx, |i, cx| i.set_text("", cx));
            self.password_repeat.update(cx, |i, cx| i.set_text("", cx));
            self.recovery_input.update(cx, |i, cx| i.set_text("", cx));
        }
        match result {
            Ok(AccountOutcome::Prepared(pending)) => {
                self.account.pending = Some(pending);
                self.account.saved = false;
                self.account.copied = false;
            }
            Ok(AccountOutcome::Opened(collection, settings, binding, root)) => {
                self.account.pending = None;
                self.account_email
                    .update(cx, |i, cx| i.set_text(&binding.account.email, cx));
                self.root = Some(root);
                self.account.signed_out = false;
                self.account.binding = Some(binding);
                self.account.mode = AccountMode::Unlock;
                self.account.saved = false;
                self.clear_owned_clipboard(cx);
                self.settings = settings;
                self.collection = collection;
                self.unlocked = true;
                self.initialized = true;
                self.restore_from = None;
                self.notice = Some("Account connected. Your vault is stored on this device; sync is not available yet.".into());
                self.stop_codex_setup();
                self.start_codex_setup(false, cx);
                self.focus_filter_on_ready = true;
                self.refresh_search(cx);
                self.refresh_imports(cx);
                self.kick_auto_queue(cx);
            }
            Ok(AccountOutcome::AccountOnly(email)) => {
                self.account.pending = None;
                self.account.saved = false;
                self.clear_owned_clipboard(cx);
                self.account.complete = Some(email);
            }
            Err(e) => {
                self.error = Some(e);
                self.focus_password_on_ready = self.account.pending.is_none();
            }
        }
        cx.notify();
    }
    pub(super) fn account_focus(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        backwards: bool,
    ) {
        if self.account.pending.is_some() {
            if backwards {
                window.focus_prev();
            } else {
                window.focus_next();
            }
            return;
        }
        if self.account.complete.is_some() {
            cx.propagate();
            return;
        }
        let mut fields = vec![&self.account_email];
        if self.account.mode == AccountMode::Recover {
            fields.push(&self.recovery_input);
        }
        fields.push(&self.password);
        if self.account.mode != AccountMode::SignIn {
            fields.push(&self.password_repeat);
        }
        let current = fields
            .iter()
            .position(|f| f.focus_handle(cx).is_focused(window));
        let index = current
            .map(|i| {
                if backwards {
                    (i + fields.len() - 1) % fields.len()
                } else {
                    (i + 1) % fields.len()
                }
            })
            .unwrap_or(0);
        window.focus(&fields[index].focus_handle(cx));
    }
    fn copy_recovery(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = &self.account.pending else {
            return;
        };
        let value = pending.code.clone();
        cx.write_to_clipboard(ClipboardItem::new_string(value.to_string()));
        self.copied_value = Some(value);
        self.clear_clipboard_later(cx);
        self.account.copied = true;
        let generation = self.clipboard_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(2))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.clipboard_generation == generation {
                    this.account.copied = false;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn account_unlock_view(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let body = div()
            .flex()
            .flex_col()
            .gap(px(space::LG))
            .when_some(self.account.binding.as_ref(), |s, binding| {
                s.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(space::SM))
                        .child(
                            div()
                                .type_style(Type::Small)
                                .text_color(rgb(MUTED))
                                .child("Email address"),
                        )
                        // The remembered identity is display-only. Only the password receives focus.
                        .child(
                            div()
                                .h(px(layout::CONTROL_LARGE))
                                .px(px(space::MD))
                                .rounded(px(radius::STANDARD))
                                .border_1()
                                .border_color(rgb(LINE))
                                .bg(rgb(BG))
                                .flex()
                                .items_center()
                                .min_w_0()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .child(binding.account.email.clone()),
                                ),
                        ),
                )
            })
            .child(self.editor_field("Master password", self.password.clone(), window, cx))
            .when_some(self.motion.error.clone(), |s, error| {
                s.child(
                    div()
                        .type_style(Type::Small)
                        .text_color(rgb(DANGER))
                        .child(error),
                )
            })
            .when_some(self.error.clone(), |s, error| {
                s.child(
                    div()
                        .p(px(space::MD))
                        .rounded(px(radius::STANDARD))
                        .bg(rgb(DANGER_SURFACE))
                        .text_color(rgb(DANGER))
                        .type_style(Type::Small)
                        .child(error),
                )
            })
            .child(
                primary_action()
                    .id("unlock-vault")
                    .h(px(layout::CONTROL_LARGE))
                    .hover(|s| s.bg(rgb(PRIMARY_HOVER)))
                    .when(self.busy, |s| s.cursor_default())
                    .on_click(cx.listener(|this, _, window, cx| {
                        window.focus(&this.focus);
                        this.unlock_vault(cx);
                    }))
                    .child(if self.busy {
                        "Unlocking your vault…"
                    } else {
                        "Unlock"
                    })
                    .child(action_indicator(self.busy, self.motion_enabled())),
            )
            .when(self.account.binding.is_some(), |s| {
                s.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(space::SM))
                        .child(
                            div()
                                .id("unlock-recovery")
                                .py(px(space::XS))
                                .type_style(Type::Small)
                                .text_color(rgb(ACCENT))
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.set_account_mode(AccountMode::Recover, cx)
                                }))
                                .child("Forgot your master password?"),
                        )
                        .child(
                            div()
                                .id("unlock-online")
                                .py(px(space::XS))
                                .type_style(Type::Small)
                                .text_color(rgb(MUTED))
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.set_account_mode(AccountMode::SignIn, cx)
                                }))
                                .child("Sign in after a password change"),
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .gap(px(space::SM))
                    .child(
                        secondary_action()
                            .id("unlock-register")
                            .flex_1()
                            .hover(|s| s.bg(rgb(HOVER)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_account_mode(AccountMode::Register, cx)
                            }))
                            .child("Register"),
                    )
                    .when(self.account.binding.is_some(), |s| {
                        s.child(
                            secondary_action()
                                .id("unlock-logout")
                                .flex_1()
                                .hover(|s| s.bg(rgb(HOVER)))
                                .on_click(cx.listener(|this, _, _, cx| this.log_out(cx)))
                                .child("Log out"),
                        )
                    }),
            )
            .child(
                div()
                    .pt(px(space::SM))
                    .border_t_1()
                    .border_color(rgb(LINE))
                    .type_style(Type::Caption)
                    .text_color(rgb(MUTED))
                    .child("Your vault is encrypted on this device. You can unlock offline."),
            );
        self.onboarding_shell(
            0,
            "Welcome back.",
            "Enter your master password to unlock your space.",
            body.into_any_element(),
            window,
            cx,
        )
    }

    pub(super) fn account_view(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if self.provider_setup_visible() {
            return self.provider_onboarding_view(window, cx);
        }
        let pending = self.account.pending.as_ref();
        let recovering = self.account.mode == AccountMode::Recover;
        let signing_in = self.account.mode == AccountMode::SignIn;
        let complete = self.account.complete.is_some();
        let step = usize::from(pending.is_some());
        let title = if complete {
            "Your account is ready."
        } else if pending.is_some() {
            "Keep your way back in."
        } else if recovering {
            "Recover your account."
        } else if signing_in {
            "Welcome back."
        } else {
            "Create your account."
        };
        let subtitle = if complete {
            "Account access is working. Device sync is coming next."
        } else if pending.is_some() {
            "Save this code outside ME. You’ll need it to reset a forgotten master password."
        } else if recovering {
            "Use your recovery code to choose a new master password."
        } else if signing_in {
            "Your email. Your master password. One vault."
        } else {
            "Your master password is also your key to unlock ME. on every device."
        };
        let body = div().tab_group().flex().flex_col().gap(px(space::LG))
            .when(pending.is_none() && !complete, |s| s
                .child(self.editor_field("Email address", self.account_email.clone(), window, cx))
                .when(recovering, |s| s.child(self.editor_field("Recovery code", self.recovery_input.clone(), window, cx)))
                .child(self.editor_field(if recovering { "New master password" } else { "Master password" }, self.password.clone(), window, cx))
                .when(!signing_in, |s| s.child(self.editor_field("Repeat master password", self.password_repeat.clone(), window, cx)))
                .child(div().type_style(Type::Caption).text_color(rgb(MUTED)).child(
                    if self.initialized && !recovering && !signing_in { "Use your existing vault password (at least 10 characters). Your saved data stays on this device." }
                    else if signing_in { "Initial sign-in requires an internet connection." }
                    else { "At least 10 characters. Choose something long and memorable." })))
            .when_some(pending, |s, p| s
                .child(div().rounded(px(radius::STANDARD)).border_1().border_color(rgb(LINE)).overflow_hidden()
                    .child(div().px(px(space::LG)).py(px(space::SM)).flex().items_center().justify_between().gap(px(space::SM))
                        .child(eyebrow("RECOVERY CODE"))
                        .child(secondary_action().id("copy-recovery-code").tab_index(0)
                            .focus(|s| s.border_color(rgb(FOCUS)))
                            .on_action(|_: &Confirm, _, cx| cx.stop_propagation())
                            .h(px(layout::CONTROL_COMPACT)).px(px(space::MD))
                            .hover(|s| s.bg(rgb(HOVER)))
                            .on_click(cx.listener(|this, _, _, cx| this.copy_recovery(cx)))
                            .child(icon(if self.account.copied { Icon::Check } else { Icon::Copy }, IconSize::Medium, ACCENT))
                            .child(if self.account.copied { "Copied" } else { "Copy" })))
                    .child(div().p(px(space::LG)).bg(rgb(BG)).border_t_1().border_color(rgb(LINE))
                        .font_family(font::MONO).type_style(Type::Value).text_center().flex().flex_col().gap(px(space::SM))
                        .children(p.code.split('-').collect::<Vec<_>>().chunks(4).map(|line| div().child(line.join("-"))).collect::<Vec<_>>())))
                .child(div().type_style(Type::Small).text_color(rgb(MUTED)).child(
                    if recovering { "This replaces your previous code when recovery finishes. ME. will not show it again." }
                    else { "Shown only this once. Store it in a safe place: anyone with this code can recover your account." }))
                .child(checkbox(self.account.saved, "I’ve saved my recovery code")
                    .id("recovery-code-saved").tab_index(1).tab_stop(!self.busy)
                    .hover(|s| s.bg(rgb(HOVER))).focus(|s| s.bg(rgb(HOVER)))
                    .when(self.busy, |s| s.opacity(0.5).cursor_default())
                    .on_action(|_: &Confirm, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, _, cx| { if !this.busy { this.account.saved = !this.account.saved; this.error = None; cx.notify(); } }))))
            .when_some(self.account.complete.clone(), |s, email| s
                .child(div().font_family(font::MONO).type_style(Type::Body).child(email))
                .child(div().type_style(Type::Body).text_color(rgb(MUTED)).child("Your existing vault has not been downloaded. Open ME. on your original device to access your data until device sync is available.")))
            .when_some(self.error.clone(), |s, e| s.child(div().p(px(space::MD)).rounded(px(radius::STANDARD)).bg(rgb(DANGER_SURFACE)).text_color(rgb(DANGER)).type_style(Type::Small).child(e)))
            .when(!complete, |s| s.child(primary_action().id("account-continue").h(px(layout::CONTROL_LARGE))
                .when(pending.is_some(), |s| s.tab_index(2).tab_stop(!self.busy && self.account.saved)
                    .focus(|s| s.bg(rgb(PRIMARY_HOVER)))
                    .on_action(|_: &Confirm, _, cx| cx.stop_propagation()))
                .hover(|s| s.bg(rgb(PRIMARY_HOVER)))
                .when(pending.is_some() && !self.account.saved, |s| s.opacity(0.5))
                .when(self.busy, |s| s.cursor_default())
                .on_click(cx.listener(|this, _, window, cx| { window.focus(&this.focus); this.submit_account(cx); }))
                .child(if self.busy { if pending.is_some() { if recovering { "Recovering account…" } else { "Creating account…" } } else if signing_in { "Signing in…" } else if recovering { "Preparing recovery…" } else { "Securing your account…" } } else if pending.is_some() { if recovering { "Finish recovery" } else { "Create account & continue" } } else if signing_in { "Sign in" } else if recovering { "Continue recovery" } else { "Continue" })
                .child(action_indicator(self.busy, self.motion_enabled()))))
            .when(complete, |s| s.child(secondary_action().id("account-complete-back").on_click(cx.listener(|this, _, _, cx| this.set_account_mode(AccountMode::SignIn, cx))).child("Back to sign in")))
            .when(!complete, |s| s.child(div().flex().flex_col().items_center().gap(px(space::SM))
                .child(div().id("account-switch").py(px(space::XS)).type_style(Type::Small).text_color(rgb(ACCENT)).cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| { let mode = if this.account.pending.is_some() { this.account.mode } else if this.account.mode == AccountMode::Register { AccountMode::SignIn } else { AccountMode::Register }; this.set_account_mode(mode, cx); }))
                    .child(if pending.is_some() { "Back to account details" } else if self.account.mode == AccountMode::Register { "Already have an account? Sign in" } else { "Register" }))
                .when(!recovering && pending.is_none(), |s| s.child(div().id("account-recover").py(px(space::XS)).type_style(Type::Small).text_color(rgb(MUTED)).cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.set_account_mode(AccountMode::Recover, cx))).child("Forgot your master password?")))
                .when(self.account.binding.is_some(), |s| s.child(secondary_action().id("account-local-unlock").on_click(cx.listener(|this, _, _, cx| this.set_account_mode(AccountMode::Unlock, cx))).child("Back to offline unlock")))))
            .when(!self.initialized && pending.is_none() && !complete, |s| s.child(div().id("account-restore-backup").text_center().type_style(Type::Small).text_color(rgb(MUTED)).cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| this.pick_restore(cx))).child("Restore a backup")))
            .when(!complete && pending.is_none(), |s| s.child(div().pt(px(space::SM)).border_t_1().border_color(rgb(LINE)).type_style(Type::Caption).text_color(rgb(MUTED)).child("Your vault stays on this device. Device sync is coming later.")));
        self.onboarding_shell(step, title, subtitle, body.into_any_element(), window, cx)
    }
}
enum AccountOutcome {
    Prepared(Arc<PendingAccount>),
    Opened(Collection, me_core::AppSettings, AccountBinding, PathBuf),
    AccountOnly(String),
}
fn open_response(
    base: &std::path::Path,
    root: Option<&std::path::Path>,
    password: &str,
    server: &str,
    response: AccountResponse,
    session: Arc<Mutex<Option<Vault>>>,
) -> Result<AccountOutcome, String> {
    let Some(root) = root else {
        account::validate_password_envelope(password, &response).map_err(|e| e.to_string())?;
        return Ok(AccountOutcome::AccountOnly(response.account.email));
    };
    let vault = account::open_account(root, password, server, &response, false)
        .map_err(|e| e.to_string())?;
    let collection = vault.collection("", false).map_err(|e| e.to_string())?;
    let settings = vault.settings().map_err(|e| e.to_string())?;
    let binding = AccountBinding {
        account: response.account,
        server: server.into(),
    };
    device_accounts::select(base, root)?;
    *session
        .lock()
        .map_err(|_| "Could not open the local session.".to_owned())? = Some(vault);
    Ok(AccountOutcome::Opened(
        collection,
        settings,
        binding,
        root.to_owned(),
    ))
}
