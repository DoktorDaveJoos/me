use super::credential_capture::{Capture, Intent, origin};
use super::*;
use me_core::CredentialKind;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
pub(super) struct CredentialIntake {
    pub open: bool,
    sources: Vec<context_source::Source>,
    pub(super) capture: Option<Capture>,
    loading: bool,
    permission: bool,
    pub(super) permission_source: Option<context_source::Source>,
    more: bool,
    scroll: gpui::ScrollHandle,
    error: Option<String>,
    request: u64,
    cancel: Option<Arc<AtomicBool>>,
}
impl Drop for CredentialIntake {
    fn drop(&mut self) {
        if let Some(c) = &self.cancel {
            c.store(true, Ordering::SeqCst);
        }
    }
}
impl MeApp {
    pub(super) fn new_credential(
        &mut self,
        _: &NewCredential,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.app_ready() || self.busy || self.login_edit_guard(cx) {
            return;
        }
        self.page = Page::Logins;
        self.show_settings = false;
        self.show_add = false;
        self.show_onepassword = false;
        self.logins.intake = CredentialIntake {
            open: true,
            request: self.logins.intake.request + 1,
            sources: Vec::new(),
            capture: None,
            loading: false,
            permission: false,
            permission_source: None,
            more: false,
            scroll: gpui::ScrollHandle::default(),
            error: None,
            cancel: None,
        };
        self.logins
            .detail_scroll
            .set_offset(gpui::point(px(0.), px(0.)));
        self.logins.error = None;
        self.logins.notice = None;
        window.focus(&self.login_focus);
        self.logins.fill_target = None;
        self.logins.fill_after_save = false;
        if !self.logins.loaded {
            self.refresh_logins(cx);
        }
        self.list_context_sources(false, cx);
        cx.notify();
    }
    pub(super) fn close_credential_intake(&mut self, cx: &mut Context<Self>) {
        self.logins.intake = CredentialIntake {
            open: false,
            request: self.logins.intake.request + 1,
            sources: Vec::new(),
            capture: None,
            loading: false,
            permission: false,
            permission_source: None,
            more: false,
            scroll: gpui::ScrollHandle::default(),
            error: None,
            cancel: None,
        };
        cx.notify();
    }
    pub(super) fn begin_credential(
        &mut self,
        kind: CredentialKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy || self.logins.draft.is_some() {
            return;
        }
        let capture = self.logins.intake.capture.take();
        self.close_credential_intake(cx);
        self.logins.detail_request += 1;
        self.logins.detail_loading = false;
        self.logins.creating = Some(kind);
        self.logins.details = Some(me_core::credential_draft(kind));
        self.logins.arrival = Some(std::time::Instant::now());
        self.edit_login(&EditLogin, window, cx);
        if let Some(capture) = capture {
            self.apply_capture(capture, false, cx);
        }
        cx.notify();
    }
    fn apply_capture(&mut self, mut capture: Capture, existing: bool, cx: &mut Context<Self>) {
        if !existing && self.logins.creating == Some(CredentialKind::Login) {
            self.logins.fill_target = capture.fill_target.take();
        }
        let (Some(details), Some(draft)) = (&self.logins.details, &self.logins.draft) else {
            return;
        };
        if !existing
            && let Ok(u) = url::Url::parse(&capture.website)
            && let Some(host) = u.host_str()
        {
            draft.title.update(cx, |i, cx| i.set_text(host, cx));
        }
        for (index, input) in &draft.fields {
            let f = &details.fields[*index];
            let logical = if details.native_kind.is_some() {
                f.key.as_str()
            } else if f.key == "$recovery_codes" {
                "recovery_codes"
            } else if f.label == "Password" {
                "password"
            } else if f.label == "Username" {
                "username"
            } else if f.key == "/overview/url" {
                "website"
            } else {
                ""
            };
            if existing
                && !matches!(
                    (capture.intent, logical),
                    (Intent::PasswordUpdate, "password")
                        | (Intent::RecoveryCodes, "recovery_codes")
                )
            {
                continue;
            }
            let value = if logical == "website" && !capture.website.is_empty() {
                Some(capture.website.as_str())
            } else {
                capture
                    .values
                    .iter()
                    .find(|(key, _)| key == logical)
                    .map(|(_, v)| v.as_str())
            };
            let value = value.or_else(|| {
                if !existing
                    && matches!(
                        (details.native_kind, logical),
                        (Some(CredentialKind::Password), "password")
                            | (Some(CredentialKind::Api), "token")
                    )
                {
                    capture.unassigned.as_deref().map(|v| v.as_str())
                } else {
                    None
                }
            });
            if let Some(value) = value {
                // Never replace existing recovery material implicitly. Preserve it and append for review.
                let value = if existing && logical == "recovery_codes" && !f.value.is_empty() {
                    format!("{}\n{}", f.value.as_str(), value)
                } else {
                    value.into()
                };
                let value = zeroize::Zeroizing::new(value);
                input.update(cx, |i, cx| i.set_text(&value, cx));
            }
        }
        self.logins.notice = Some(format!(
            "Suggested from {}. Review every field before saving.{}",
            capture.source,
            if capture.password_present {
                " The page already contains a password. Paste it into the draft, or clear the page and start again to generate a new one."
            } else if capture.registration {
                " Registration detected. The suggested email is editable; the password was generated on this device. Save and fill does not submit the website form."
            } else if existing && capture.intent == Intent::PasswordUpdate {
                " Saving here does not change the password on the website."
            } else {
                ""
            }
        ));
    }
    fn use_capture_for_login(&mut self, item: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.logins.intake.loading || self.busy {
            return;
        }
        let Some(capture) = self.logins.intake.capture.take() else {
            return;
        };
        self.close_credential_intake(cx);
        self.logins.detail_request += 1;
        let request = self.logins.detail_request;
        let generation = self.generation;
        self.logins.detail_loading = true;
        self.logins.details = None;
        self.logins.selected = Some(item);
        let session = self.session.clone();
        let task = cx.background_executor().spawn(async move {
            session
                .lock()
                .map_err(|_| me_core::Error::Format)?
                .as_ref()
                .ok_or(me_core::Error::Format)?
                .login_details(item)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation || this.logins.detail_request != request {
                    return;
                }
                this.logins.detail_loading = false;
                match result {
                    Ok(details) => {
                        this.logins.details = Some(details);
                        this.edit_login(&EditLogin, window, cx);
                        this.apply_capture(capture, true, cx);
                    }
                    Err(_) => {
                        this.logins.intake.open = true;
                        this.logins.intake.capture = Some(capture);
                        this.logins.intake.error = Some(
                            "Couldn't open this item. Your captured draft is still here.".into(),
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn paste_credential_context(&mut self, cx: &mut Context<Self>) {
        if self.logins.intake.loading {
            return;
        }
        let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else {
            self.logins.intake.error = Some("Copy the relevant text, then choose Paste.".into());
            cx.notify();
            return;
        };
        self.logins.intake.permission_source = None;
        let text = zeroize::Zeroizing::new(text);
        if text.len() > 65536 {
            self.logins.intake.error = Some("Paste up to 64 KB of relevant text.".into());
        } else {
            self.logins.intake.capture = Some(Capture::from_text("Clipboard".into(), &text, ""));
            self.logins.intake.sources.clear();
            self.logins.intake.error = None;
        }
        cx.notify();
    }
    fn list_context_sources(&mut self, request_permission: bool, cx: &mut Context<Self>) {
        if self.logins.intake.loading {
            return;
        }
        self.logins.intake.loading = true;
        self.logins.intake.error = None;
        self.logins.intake.request += 1;
        let request = self.logins.intake.request;
        let generation = self.generation;
        let task = cx
            .background_executor()
            .spawn(async move { context_source::sources(request_permission) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation || this.logins.intake.request != request {
                    return;
                }
                this.logins.intake.loading = false;
                match result {
                    Ok(s) => {
                        this.logins.intake.sources = s.windows;
                        this.logins.intake.permission = s.permission;
                        this.logins.intake.more = s.more;
                    }
                    Err(e) => this.logins.intake.error = Some(e.into()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn capture_context_source(
        &mut self,
        source: context_source::Source,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.logins.intake.loading {
            return;
        }
        self.logins.intake.loading = true;
        self.logins.intake.error = None;
        self.logins.intake.request += 1;
        let request = self.logins.intake.request;
        let generation = self.generation;
        self.logins.intake.permission_source = None;
        let selected_source = source.clone();
        let email = self
            .account
            .binding
            .as_ref()
            .map(|b| b.account.email.clone())
            .unwrap_or_default();
        let cancel = Arc::new(AtomicBool::new(false));
        self.logins.intake.cancel = Some(cancel.clone());
        let task = cx.background_executor().spawn(async move {
            let mut capture = context_source::capture(&source, &cancel)?;
            // Clear forms are classified locally. TypeSafe resolves only ambiguous label concepts.
            if capture.intent == Intent::Unknown
                && !capture.signals.is_empty()
                && !cancel.load(Ordering::SeqCst)
                && let Ok(Some(key)) =
                    me_agent::typesafe::credential_intent(&capture.signals, &cancel)
            {
                capture.intent = match key.as_str() {
                    "update_password" => Intent::PasswordUpdate,
                    "recovery_codes" => Intent::RecoveryCodes,
                    key => CredentialKind::from_key(key)
                        .map(Intent::Create)
                        .unwrap_or(Intent::Unknown),
                };
            }
            capture
                .prepare_registration(&email)
                .map_err(|_| "Couldn't prepare the registration draft.")?;
            Ok::<_, context_source::CaptureError>(capture)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation || this.logins.intake.request != request {
                    return;
                }
                this.logins.intake.loading = false;
                this.logins.intake.cancel = None;
                match result {
                    Ok(capture) => {
                        let intent = capture.intent;
                        this.logins.intake.capture = Some(capture);
                        if let Intent::Create(kind) = intent {
                            this.begin_credential(kind, window, cx);
                        }
                    }
                    Err(context_source::CaptureError::Accessibility) => {
                        this.logins.intake.permission_source = Some(selected_source);
                        this.logins
                            .intake
                            .scroll
                            .set_offset(gpui::point(px(0.), px(0.)));
                    }
                    Err(context_source::CaptureError::Unavailable(e)) => {
                        this.logins.intake.error = Some(e.into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn open_credential_accessibility_settings(&mut self, cx: &mut Context<Self>) {
        if self.logins.intake.loading {
            return;
        }
        self.logins.intake.loading = true;
        let generation = self.generation;
        let request = self.logins.intake.request;
        let task = cx
            .background_executor()
            .spawn(async { context_source::request_accessibility() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation || this.logins.intake.request != request {
                    return;
                }
                this.logins.intake.loading = false;
                if let Err(error) = result {
                    this.logins.intake.error = Some(error.into());
                }
                cx.open_url(
                    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
                );
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn generate_login_password(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(input) = self.logins.draft.as_ref().and_then(|d| {
            d.fields
                .iter()
                .find(|(i, _)| *i == index)
                .map(|(_, i)| i.clone())
        }) else {
            return;
        };
        // OS random generation is bounded and runs off the UI thread.
        let generation = self.generation;
        let request = self.logins.detail_request;
        let task = cx
            .background_executor()
            .spawn(async { me_core::generate_credential_password() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation
                    || this.logins.detail_request != request
                    || this.logins.draft.is_none()
                    || this.busy
                {
                    return;
                }
                match result {
                    Ok(secret) => {
                        input.update(cx, |i, cx| {
                            i.set_concealed(true, cx);
                            i.set_text(&secret, cx)
                        });
                        this.logins.revealed.remove(&index);
                        this.logins.notice = Some(
                            "Generated locally · 20 random characters. Save after reviewing."
                                .into(),
                        );
                    }
                    Err(_) => {
                        this.logins.error = Some("Couldn't generate a password. Try again.".into())
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn credential_intake_modal(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let state = &self.logins.intake;
        let capture = state.capture.as_ref();
        let update = capture
            .is_some_and(|c| matches!(c.intent, Intent::PasswordUpdate | Intent::RecoveryCodes));
        let matches: Vec<_> = self
            .logins
            .items
            .iter()
            .filter(|i| {
                i.category == "001"
                    && !i.archived
                    && capture.is_some_and(|c| {
                        !c.website.is_empty()
                            && origin(&i.website).as_deref() == Some(c.website.as_str())
                    })
            })
            .take(20)
            .collect();
        let content = div().id("credential-chooser-scroll").track_scroll(&state.scroll)
            .flex_1().min_h_0().overflow_y_scroll().flex().flex_col().gap(px(space::XL))
            .when_some(state.permission_source.as_ref(), |s,source| s.child(div().p(px(space::LG)).rounded(px(radius::STANDARD)).bg(rgb(WARNING_SURFACE)).flex().flex_col().gap(px(space::MD))
                .child(div().type_style(Type::Section).text_color(rgb(INK)).child("Allow access to this window"))
                .child(div().type_style(Type::Small).text_color(rgb(WARNING)).child(format!("Enable ME. in System Settings → Privacy & Security → Accessibility to read {}. Your selected window is kept for retry.",source.name)))
                .child(div().type_style(Type::Small).text_color(rgb(WARNING)).child("Already enabled after an update? Quit ME., remove its old Accessibility entry with −, then add this copy with + and reopen it."))
                .child(div().flex().flex_wrap().gap(px(space::SM))
                    .child(secondary_action().id("open-credential-accessibility").on_click(cx.listener(|this,_,_,cx|this.open_credential_accessibility_settings(cx))).child("Open Settings"))
                    .child(primary_action().id("retry-credential-accessibility").on_click(cx.listener({let source=source.clone();move|this,_,window,cx|this.capture_context_source(source.clone(),window,cx)})).child("Try again")))))
            .child(div().flex().flex_col().gap(px(space::SM))
                .children(CredentialKind::ALL.chunks(2).map(|row| div().flex().gap(px(space::SM)).children(row.iter().copied().map(|kind| {
                    div().id(gpui::SharedString::from(format!("credential-type-{}", kind.key())))
                        .flex_1().min_w_0().p(px(space::MD)).rounded(px(radius::STANDARD)).bg(rgb(BG)).border_1().border_color(rgb(LINE))
                        .cursor_pointer().hover(|s| s.bg(rgb(HOVER)).border_color(rgb(FOCUS)))
                        .on_click(cx.listener(move |this, _, window, cx| this.begin_credential(kind, window, cx)))
                        .flex().gap(px(space::MD))
                        .child(icon(match kind { CredentialKind::Login => Icon::User, CredentialKind::Password => Icon::Key, CredentialKind::Api => Icon::Hash, CredentialKind::Ssh => Icon::Key, CredentialKind::Wifi => Icon::Grid, CredentialKind::Server => Icon::Folder }, IconSize::Large, ACCENT))
                        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(space::XS)).child(div().type_style(Type::Label).child(kind.label())).child(div().type_style(Type::Small).text_color(rgb(MUTED)).child(kind.description())))
                })))))
            .when_some(capture, |s,c| s.child(div().p(px(space::LG)).rounded(px(radius::STANDARD)).border_1().border_color(rgb(LINE)).flex().flex_col().gap(px(space::SM))
                .child(eyebrow(format!("Detected from {}", c.source))).child(heading(c.intent.label()))
                .when(!c.website.is_empty(), |s| s.child(div().type_style(Type::Small).font_family(font::MONO).child(c.website.clone())))
                .when(!update && c.intent != Intent::Unknown, |s| { let kind = c.intent.kind(); s.child(primary_action().id("review-capture").on_click(cx.listener(move |this,_,window,cx| this.begin_credential(kind,window,cx))).child("Review draft")) })
                .when(update, |s| s.child(div().type_style(Type::Small).text_color(rgb(MUTED)).child("Choose the login to update. Website matches are suggestions.")))
                .children(if update {matches.clone()} else {Vec::new()}.into_iter().map(|i| { let id=i.id; secondary_action().id(("context-match",id)).on_click(cx.listener(move |this,_,window,cx| this.use_capture_for_login(id,window,cx))).child(format!("{} · {}",i.title,i.vault)) }))
                .when(update, |s| s.child(secondary_action().id("new-captured-login").on_click(cx.listener(|this,_,window,cx| this.begin_credential(CredentialKind::Login,window,cx))).child("Create a new login instead")))))
            .when(update, |s| s.child(div().flex().flex_col().gap(px(space::SM)).child(eyebrow("Other logins"))
                .children(self.logins.items.iter().filter(|i| i.category=="001" && !i.archived && !matches.iter().any(|m|m.id==i.id)).take(50).map(|i|{let id=i.id;secondary_action().id(("context-other",id)).on_click(cx.listener(move|this,_,window,cx|this.use_capture_for_login(id,window,cx))).child(format!("{} · {}",i.title,i.vault))}))))
            .child(div().flex().items_center().justify_between().gap(px(space::MD))
                .child(div().flex().flex_col().gap(px(space::XS)).child(div().type_style(Type::Section).child("Start from an open window")).child(div().type_style(Type::Small).text_color(rgb(MUTED)).child("Choose the page you're working on. ME. prepares the matching draft.")))
                .child(secondary_action().id("refresh-window-previews").on_click(cx.listener(|this,_,_,cx|this.list_context_sources(false,cx))).child("Refresh")))
            .when(state.loading, |s| s.child(div().flex().gap(px(space::SM)).child(action_indicator(true, self.motion_enabled())).child(div().type_style(Type::Small).text_color(rgb(MUTED)).child(if state.cancel.is_some() { "Preparing your draft…" } else if state.permission_source.is_some() { "Opening Accessibility settings…" } else { "Loading window previews…" }))))
            .when_some(state.error.clone(), |s,e| s.child(div().p(px(space::MD)).rounded(px(radius::STANDARD)).bg(rgb(WARNING_SURFACE)).type_style(Type::Small).text_color(rgb(WARNING)).child(e)))
            .when(!state.loading && !state.permission && cfg!(target_os="macos"), |s| s.child(div().p(px(space::LG)).rounded(px(radius::STANDARD)).bg(rgb(BG)).flex().flex_col().gap(px(space::MD))
                .child(div().type_style(Type::Small).text_color(rgb(MUTED)).child("Allow Screen Recording to see window previews. Images stay on this device and are discarded when this chooser closes."))
                .child(secondary_action().id("enable-window-previews").on_click(cx.listener(|this,_,window,cx|this.open_permissions(&OpenPermissions,window,cx))).child("Enable window previews"))))
            .when(!state.loading && state.permission && state.sources.is_empty(), |s| s.child(div().type_style(Type::Small).text_color(rgb(MUTED)).child("No other visible windows. Open a page, then refresh.")))
            .children(state.sources.chunks(2).map(|row| div().flex().gap(px(space::MD)).children(row.iter().enumerate().map(|(index,source)| {
                let source = source.clone();
                div().id(gpui::SharedString::from(format!("preview-{}-{index}",source.identity["id"])))
                    .flex_1().min_w_0().rounded(px(radius::STANDARD)).overflow_hidden().border_1().border_color(rgb(LINE)).bg(rgb(BG)).cursor_pointer().hover(|s|s.border_color(rgb(ACCENT)))
                    .on_click(cx.listener({ let source=source.clone(); move |this,_,window,cx|this.capture_context_source(source.clone(),window,cx) }))
                    .flex().flex_col()
                    .child(div().w_full().h(px(150.)).overflow_hidden().flex().items_center().justify_center().child(if let Some(image)=source.preview { gpui::img(image).w_full().h_full().object_fit(gpui::ObjectFit::Contain).into_any_element() } else { div().type_style(Type::Small).text_color(rgb(MUTED)).child("Preview unavailable").into_any_element() }))
                    .child(div().p(px(space::MD)).flex().flex_col().gap(px(space::XS)).child(div().type_style(Type::Body).truncate().child(source.title)).child(div().type_style(Type::Caption).text_color(rgb(MUTED)).child(source.name)))
            }))))
            .when(state.more, |s|s.child(div().type_style(Type::Caption).text_color(rgb(MUTED)).child("Showing 12 visible windows. Bring the window you want forward and refresh.")))
            .child(div().flex().flex_wrap().items_center().gap(px(space::MD))
                .child(secondary_action().id("capture-paste").on_click(cx.listener(|this,_,_,cx|this.paste_credential_context(cx))).child("Paste copied details"))
                .child(div().flex_1().type_style(Type::Caption).text_color(rgb(MUTED)).child("Passwords are generated locally. TypeSafe can classify field labels; screenshots and values stay here.")));
        self.overlay(cx)
            .p(px(space::LG))
            .child(
                modal_panel(760., self.motion_enabled())
                    .h((window.viewport_size().height - px(space::SECTION)).min(px(720.)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(space::LG))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(space::MD))
                                    .child(icon(Icon::Honeycomb, IconSize::Brand, ACCENT))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap(px(space::XS))
                                            .child(heading("New item"))
                                            .child(
                                                div()
                                                    .type_style(Type::Small)
                                                    .text_color(rgb(MUTED))
                                                    .child("What would you like to save?"),
                                            ),
                                    ),
                            )
                            .child(
                                secondary_action()
                                    .id("cancel-intake")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.close_credential_intake(cx)
                                    }))
                                    .child("Cancel"),
                            ),
                    )
                    .child(content),
            )
            .into_any_element()
    }
}
