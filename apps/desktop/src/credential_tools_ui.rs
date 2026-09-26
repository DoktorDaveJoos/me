//! Local editor tools. Candidates and generated secrets never leave the vault/device.
use super::*;
use crate::assets;
use gpui::Div;
use me_core::{CredentialSuggestions, PasswordOptions};

pub(super) struct CredentialTools {
    pub suggestions: CredentialSuggestions,
    pub loading: bool,
    pub error: bool,
    pub identity_open: Option<usize>,
    pub generator_open: Option<usize>,
    pub options: PasswordOptions,
    pub length: Entity<TextInput>,
    pub generating: bool,
    password_error: Option<String>,
    generated: Option<GeneratedPassword>,
    generation_sequence: usize,
    generator_epoch: u64,
    preview_revealed: bool,
    preview_copied: bool,
    requests: PreviewRequests,
    controls: Vec<gpui::FocusHandle>,
    workshop_scroll: gpui::ScrollHandle,
    _length_subscription: gpui::Subscription,
}
// A modal candidate is independent of the editor until explicitly applied.
struct GeneratedPassword {
    index: usize,
    options: PasswordOptions,
    value: zeroize::Zeroizing<String>,
}
impl GeneratedPassword {
    fn matches(&self, index: usize, options: PasswordOptions) -> bool {
        self.index == index && self.options == options
    }
}
/// Accept only the newest recipe request, even if recipes change A → B → A.
#[derive(Default)]
struct PreviewRequests {
    sequence: u64,
    recipe: Option<PasswordOptions>,
}
impl PreviewRequests {
    fn request(&mut self, recipe: Option<PasswordOptions>, force: bool) -> Option<u64> {
        if !force && self.recipe == recipe {
            return None;
        }
        self.sequence = self.sequence.wrapping_add(1);
        self.recipe = recipe;
        Some(self.sequence)
    }
    fn accepts(&self, sequence: u64, recipe: PasswordOptions) -> bool {
        self.sequence == sequence && self.recipe == Some(recipe)
    }
}
impl CredentialTools {
    fn recipe(&self, cx: &Context<MeApp>) -> Option<PasswordOptions> {
        let length = self.length.read(cx).content.trim().parse::<usize>().ok()?;
        let o = PasswordOptions {
            length,
            ..self.options
        };
        ((8..=128).contains(&length) && (o.uppercase || o.lowercase || o.digits || o.symbols))
            .then_some(o)
    }
    fn candidate(&self, cx: &Context<MeApp>) -> Option<&GeneratedPassword> {
        if self.generating {
            return None;
        }
        let index = self.generator_open?;
        let recipe = self.recipe(cx)?;
        self.generated.as_ref().filter(|g| g.matches(index, recipe))
    }
    pub fn new(cx: &mut Context<MeApp>) -> Self {
        let length = cx.new(|cx| {
            let mut input = TextInput::new("8–128", cx);
            input.set_text("20", cx);
            input
        });
        let subscription = cx.observe(&length, |this, _, cx| {
            this.refresh_password_preview(cx);
            cx.notify();
        });
        Self {
            suggestions: CredentialSuggestions::default(),
            loading: true,
            error: false,
            identity_open: None,
            generator_open: None,
            options: PasswordOptions::default(),
            length,
            generating: false,
            password_error: None,
            generated: None,
            generation_sequence: 0,
            generator_epoch: 0,
            preview_revealed: false,
            preview_copied: false,
            requests: PreviewRequests::default(),
            controls: (0..13).map(|_| cx.focus_handle()).collect(),
            workshop_scroll: gpui::ScrollHandle::default(),
            _length_subscription: subscription,
        }
    }
}
impl MeApp {
    pub(super) fn load_credential_suggestions(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = &self.logins.draft else {
            return;
        };
        let draft_id = draft.title.entity_id();
        let generation = self.generation;
        let session = self.session.clone();
        let task = cx.background_executor().spawn(async move {
            session
                .lock()
                .map_err(|_| me_core::Error::Format)?
                .as_ref()
                .ok_or(me_core::Error::Format)?
                .credential_suggestions()
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                let Some(draft) = &mut this.logins.draft else {
                    return;
                };
                if draft.title.entity_id() != draft_id {
                    return;
                }
                draft.tools.loading = false;
                match result {
                    Ok(suggestions) => draft.tools.suggestions = suggestions,
                    Err(_) => draft.tools.error = true,
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn password_generator_open(&self) -> Option<usize> {
        self.logins
            .draft
            .as_ref()
            .and_then(|d| d.tools.generator_open)
    }
    pub(super) fn toggle_password_generator(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy || !self.app_ready() {
            return;
        }
        if self.password_generator_open().is_some() {
            self.close_password_generator(window, cx);
            return;
        }
        if let Some(draft) = &mut self.logins.draft {
            if !draft.fields.iter().any(|(i, _)| *i == index) {
                return;
            }
            let tools = &mut draft.tools;
            tools.generator_open = Some(index);
            tools
                .workshop_scroll
                .set_offset(gpui::point(px(0.), px(0.)));
            tools.generator_epoch = tools.generator_epoch.wrapping_add(1);
            tools.generated = None;
            tools.generating = false;
            tools.password_error = None;
            tools.preview_revealed = true;
            tools.preview_copied = false;
            window.focus(&tools.length.focus_handle(cx));
            self.generate_login_password(index, true, cx);
            cx.notify();
        }
    }
    pub(super) fn close_password_generator(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(draft) = &mut self.logins.draft {
            let index = draft.tools.generator_open.take();
            draft.tools.generator_epoch = draft.tools.generator_epoch.wrapping_add(1);
            draft.tools.generated = None;
            draft.tools.generating = false;
            draft.tools.preview_revealed = false;
            draft.tools.preview_copied = false;
            draft.tools.password_error = None;
            if let Some((_, input)) = draft.fields.iter().find(|(i, _)| Some(*i) == index) {
                window.focus(&input.focus_handle(cx));
            }
            cx.notify();
        }
    }
    fn refresh_password_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(index) = self.password_generator_open() {
            self.generate_login_password(index, false, cx);
        }
    }
    fn generate_login_password(&mut self, index: usize, force: bool, cx: &mut Context<Self>) {
        if self.busy || !self.app_ready() {
            return;
        }
        let Some(draft) = &mut self.logins.draft else {
            return;
        };
        if draft.tools.generator_open != Some(index) {
            return;
        }
        let recipe = draft.tools.recipe(cx);
        let Some(request) = draft.tools.requests.request(recipe, force) else {
            return;
        };
        draft.tools.generated = None;
        draft.tools.preview_copied = false;
        draft.tools.password_error = None;
        draft.tools.generating = recipe.is_some();
        let Some(options) = recipe else {
            cx.notify();
            return;
        };
        let draft_id = draft.title.entity_id();
        let epoch = draft.tools.generator_epoch;
        let generation = self.generation;
        let task = cx
            .background_executor()
            .spawn(async move { me_core::generate_password(options) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                let Some(draft) = &mut this.logins.draft else {
                    return;
                };
                if draft.title.entity_id() != draft_id
                    || draft.tools.generator_epoch != epoch
                    || draft.tools.generator_open != Some(index)
                    || !draft.tools.requests.accepts(request, options)
                    || draft.tools.recipe(cx) != Some(options)
                {
                    return;
                }
                draft.tools.generating = false;
                match result {
                    Ok(secret) => {
                        draft.tools.generation_sequence =
                            draft.tools.generation_sequence.wrapping_add(1);
                        draft.tools.generated = Some(GeneratedPassword {
                            index,
                            options,
                            value: secret,
                        });
                    }
                    Err(error) => draft.tools.password_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn use_generated_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || !self.app_ready() {
            return;
        }
        let Some(draft) = &mut self.logins.draft else {
            return;
        };
        let Some(candidate) = draft.tools.candidate(cx) else {
            return;
        };
        let index = candidate.index;
        let value = candidate.value.clone();
        let Some((_, input)) = draft.fields.iter().find(|(i, _)| *i == index) else {
            return;
        };
        input.update(cx, |input, cx| {
            input.set_concealed(true, cx);
            input.set_text(&value, cx);
        });
        self.logins.revealed.remove(&index);
        self.close_password_generator(window, cx);
        self.logins.notice = Some("Password added to the draft. Save the item to keep it.".into());
        cx.notify();
    }
    fn reveal_password_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(draft) = &mut self.logins.draft {
            draft.tools.preview_revealed = !draft.tools.preview_revealed;
            cx.notify();
        }
    }
    fn password_workshop_action(
        &mut self,
        action: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy || !self.app_ready() {
            return;
        }
        let Some(index) = self.password_generator_open() else {
            return;
        };
        let tools = &mut self.logins.draft.as_mut().unwrap().tools;
        if let Some(focus) = tools.controls.get(action) {
            window.focus(focus);
        }
        match action {
            0..=2 => {
                tools.length.update(cx, |i, cx| {
                    i.set_text(&[12, 20, 32][action].to_string(), cx)
                });
            }
            3..=6 => {
                let o = &mut tools.options;
                let value = match action {
                    3 => &mut o.uppercase,
                    4 => &mut o.lowercase,
                    5 => &mut o.digits,
                    _ => &mut o.symbols,
                };
                *value = !*value;
            }
            7 => tools.options.exclude_ambiguous = !tools.options.exclude_ambiguous,
            8 => self.generate_login_password(index, true, cx),
            9 => self.reveal_password_preview(cx),
            10 => {
                if let Some(candidate) = tools.candidate(cx) {
                    let value = candidate.value.clone();
                    cx.write_to_clipboard(ClipboardItem::new_string(value.to_string()));
                    self.copied_value = Some(value);
                    tools.preview_copied = true;
                    self.clear_clipboard_later(cx);
                }
            }
            11 => self.close_password_generator(window, cx),
            12 => self.use_generated_password(window, cx),
            _ => {}
        }
        if action <= 7 {
            self.refresh_password_preview(cx);
        }
        cx.notify();
    }
    fn password_workshop_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // GPUI dispatches Enter/Space clicks to focused buttons on key-up.
        // Only the text field needs a custom Enter action; otherwise toggles run twice.
        if let Some(draft) = &self.logins.draft
            && draft.tools.length.focus_handle(cx).is_focused(window)
            && let Some(index) = draft.tools.generator_open
        {
            self.generate_login_password(index, true, cx);
        }
    }
    pub(super) fn password_workshop_tab(
        &self,
        backwards: bool,
        window: &mut Window,
        cx: &Context<Self>,
    ) {
        let Some(draft) = &self.logins.draft else {
            return;
        };
        let handles: Vec<_> = std::iter::once(draft.tools.length.focus_handle(cx))
            .chain(draft.tools.controls.iter().cloned())
            .collect();
        let current = handles
            .iter()
            .position(|h| h.is_focused(window))
            .unwrap_or(0);
        let next = if backwards {
            (current + handles.len() - 1) % handles.len()
        } else {
            (current + 1) % handles.len()
        };
        window.focus(&handles[next]);
        if matches!(next, 10 | 11) {
            draft
                .tools
                .workshop_scroll
                .set_offset(gpui::point(px(0.), px(0.)));
        }
    }
    pub(super) fn password_generator_modal(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let tools = &self.logins.draft.as_ref().unwrap().tools;
        let recipe = tools.recipe(cx);
        let length = tools.length.read(cx).content.trim().parse::<usize>().ok();
        let candidate = tools.candidate(cx);
        let ready = candidate.is_some();
        let o = tools.options;
        let choices = [o.uppercase, o.lowercase, o.digits, o.symbols];
        let validation = if !length.is_some_and(|n| (8..=128).contains(&n)) {
            Some("Enter a whole number from 8 to 128.")
        } else if !choices.iter().any(|&on| on) {
            Some("Choose at least one character type below.")
        } else {
            None
        };
        let message = if tools.generating {
            "Updating your password…".to_owned()
        } else if let Some(g) = candidate {
            format!("{} characters · generated on this device", g.value.len())
        } else {
            "Your password appears here once the options are valid.".to_owned()
        };
        let content = div().id("password-workshop-content").track_scroll(&tools.workshop_scroll).min_h_0().overflow_y_scroll().flex().flex_col().gap(px(space::LG))
            .child(div().p(px(space::LG)).rounded(px(radius::STANDARD)).border_1().border_color(rgb(FOCUS)).bg(rgb(BG))
                .flex().flex_col().gap(px(space::SM))
                .child(div().flex().items_center().justify_between()
                    .child(eyebrow("YOUR NEW PASSWORD"))
                    .child(div().flex().gap(px(space::XS))
                        .child(icon_action("reveal-password-preview", if tools.preview_revealed { Icon::EyeOff } else { Icon::Eye }, if tools.preview_revealed { "Hide password" } else { "Show password" })
                            .track_focus(&tools.controls[9]).focus(|s|s.bg(rgb(HOVER)))
                            .when(!ready, |s|s.opacity(0.5))
                            .on_click(cx.listener(|this,_,window,cx|this.password_workshop_action(9,window,cx))))
                        .child(icon_action("copy-password-preview", Icon::Copy, "Copy for 30 seconds")
                            .track_focus(&tools.controls[10]).focus(|s|s.bg(rgb(HOVER)))
                            .when(!ready, |s|s.opacity(0.5))
                            .on_click(cx.listener(|this,_,window,cx|this.password_workshop_action(10,window,cx))))))
                .child(div().id("password-preview-value").w_full().overflow_x_scroll().type_style(Type::Value).font_family(font::MONO)
                    .child(match candidate { Some(g) if tools.preview_revealed => g.value.to_string(), Some(_) => "••••••••••••••••".into(), None => "—".into() }))
                .child(div().flex().items_center().gap(px(space::SM)).type_style(Type::Caption).text_color(rgb(MUTED))
                    .when(ready, |s|s.child(assets::icon(Icon::Check,IconSize::Small,SUCCESS)))
                    .child(if tools.preview_copied { "Copied for 30 seconds.".to_owned() } else { message })))
            .when_some(validation, |s, message|s.child(div().type_style(Type::Small).text_color(rgb(DANGER)).child(message)))
            .child(div().flex().items_center().gap(px(space::SM))
                .child(div().flex_1().type_style(Type::Small).child("Length · 8–128"))
                .children([12,20,32].into_iter().enumerate().map(|(n,preset)|secondary_action().id(("password-length",preset)).h(px(layout::CONTROL_COMPACT)).px(px(space::SM))
                    .track_focus(&tools.controls[n]).focus(|s|s.border_color(rgb(FOCUS)))
                    .when(length == Some(preset),|s|s.border_color(rgb(ACCENT)).text_color(rgb(ACCENT)))
                    .on_click(cx.listener(move|this,_,window,cx|this.password_workshop_action(n,window,cx))).child(preset.to_string())))
                .child(div().w(px(64.)).child(self.login_input_frame(&tools.length,false,cx))))
            .child(div().flex().flex_col().gap(px(space::SM))
                .child(div().flex().items_center().justify_between().child(eyebrow("INCLUDE CHARACTERS"))
                    .child(div().type_style(Type::Caption).text_color(rgb(MUTED)).child("Click to toggle")))
                .child(div().flex().gap(px(space::SM)).children([("A–Z","Uppercase"),("a–z","Lowercase"),("0–9","Numbers"),("!@#","Symbols")].into_iter().enumerate().map(|(key,(sample,label))| {
                let checked = choices[key];
                div().id(("password-option",key)).flex_1().min_w_0().rounded(px(radius::STANDARD)).py(px(space::SM))
                    .border_1().border_color(rgb(if checked { FOCUS } else { LINE })).bg(rgb(if checked { SURFACE } else { BG }))
                    .track_focus(&tools.controls[key+3]).focus(|s|s.bg(rgb(HOVER))).hover(|s|s.bg(rgb(HOVER))).cursor_pointer()
                    .on_click(cx.listener(move|this,_,window,cx|this.password_workshop_action(key+3,window,cx)))
                    .flex().flex_col().items_center().gap(px(space::XS))
                    .child(motion::password_cell(sample,checked,key,tools.generation_sequence,self.motion_enabled()))
                    .child(div().type_style(Type::Caption).child(label))
                    .child(div().flex().items_center().gap(px(space::XS)).type_style(Type::Caption).text_color(rgb(if checked { ACCENT } else { MUTED }))
                        .when(checked, |s|s.child(assets::icon(Icon::Check,IconSize::Small,ACCENT))).child(if checked { "On" } else { "Off" }))
            }))))
            .child(div().id("password-ambiguity").track_focus(&tools.controls[7]).focus(|s|s.bg(rgb(HOVER)))
                .on_click(cx.listener(|this,_,window,cx|this.password_workshop_action(7,window,cx)))
                .child(checkbox(o.exclude_ambiguous,"Avoid lookalikes · 0 O 1 I l")))
            .child(div().type_style(Type::Caption).text_color(rgb(MUTED)).child(if self.logins.creating.is_some() {
                "Use password adds it to your draft. Save the item when you’re ready."
            } else { "Use password updates your draft. Save in ME, then update the password on the website." }))
            .when_some(tools.password_error.clone(),|s,error|s.child(div().type_style(Type::Small).text_color(rgb(DANGER)).child(error)));
        self.overlay(cx)
            .p(px(space::LG))
            .on_drop(|_: &ExternalPaths, _, cx| cx.stop_propagation())
            .child(
                modal_panel(560., self.motion_enabled())
                    .id("password-workshop-modal")
                    .max_h(window.viewport_size().height - px(space::SECTION))
                    .gap(px(space::LG))
                    .on_action(cx.listener(|this, _: &Confirm, window, cx| {
                        cx.stop_propagation();
                        this.password_workshop_confirm(window, cx)
                    }))
                    .on_action(|_: &SaveLogin, _, cx| cx.stop_propagation())
                    .on_action(cx.listener(|this, _: &NextField, window, cx| {
                        cx.stop_propagation();
                        this.password_workshop_tab(false, window, cx)
                    }))
                    .on_action(cx.listener(|this, _: &PreviousField, window, cx| {
                        cx.stop_propagation();
                        this.password_workshop_tab(true, window, cx)
                    }))
                    .child(
                        div().flex_shrink_0().flex().items_center().gap(px(space::MD))
                            .child(div().size(px(layout::CONTROL_LARGE)).flex_shrink_0()
                                .rounded(px(radius::STANDARD)).bg(rgb(HOVER)).flex().items_center().justify_center()
                                .child(assets::icon(Icon::Honeycomb,IconSize::Large,ACCENT)))
                            .child(div().flex().flex_col().gap(px(space::XS))
                                .child(heading("Generate password"))
                                .child(div().type_style(Type::Small).text_color(rgb(MUTED))
                                    .child("Adjust the options. Your password updates automatically."))),
                    )
                    .child(content)
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap(px(space::SM))
                            .child(
                                secondary_action()
                                    .id("cancel-password-workshop")
                                    .track_focus(&tools.controls[11])
                                    .focus(|s| s.border_color(rgb(FOCUS)))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.password_workshop_action(11, window, cx)
                                    }))
                                    .child("Cancel"),
                            )
                            .child(div().flex_1())
                            .child(
                                secondary_action()
                                    .id("regenerate-password")
                                    .track_focus(&tools.controls[8])
                                    .focus(|s| s.border_color(rgb(FOCUS)))
                                    .when(recipe.is_none() || tools.generating, |s| {
                                        s.opacity(0.5).cursor_default()
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.password_workshop_action(8, window, cx)
                                    }))
                                    .child(if tools.generating {
                                        "Generating…"
                                    } else if ready {
                                        "Generate again"
                                    } else {
                                        "Generate"
                                    }),
                            )
                            .child(
                                primary_action()
                                    .id("use-generated-password")
                                    .track_focus(&tools.controls[12])
                                    .focus(|s| s.bg(rgb(PRIMARY_HOVER)))
                                    .when(!ready, |s| s.opacity(0.5).cursor_default())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.password_workshop_action(12, window, cx)
                                    }))
                                    .child("Use password"),
                            ),
                    ),
            )
    }
    pub(super) fn identity_suggestions(
        &self,
        index: usize,
        input: &Entity<TextInput>,
        cx: &mut Context<Self>,
    ) -> Div {
        let tools = &self.logins.draft.as_ref().unwrap().tools;
        let expanded = tools.identity_open == Some(index);
        let query = input.read(cx).content.to_lowercase();
        let candidates: Vec<_> = tools
            .suggestions
            .identities
            .iter()
            .enumerate()
            .filter(|(_, i)| expanded || i.value.to_lowercase().contains(&query))
            .collect();
        let mut body = div().flex().flex_col().gap(px(space::SM));
        if tools.loading {
            return body.child(
                div()
                    .type_style(Type::Caption)
                    .text_color(rgb(MUTED))
                    .child("Loading saved emails and usernames…"),
            );
        }
        if tools.error {
            return body.child(
                div()
                    .type_style(Type::Caption)
                    .text_color(rgb(MUTED))
                    .child("Couldn't load saved identities. You can still type one."),
            );
        }
        body = body.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(space::SM))
                .child(
                    div()
                        .flex_1()
                        .type_style(Type::Caption)
                        .text_color(rgb(MUTED))
                        .child("From your saved logins · type to filter"),
                )
                .child(
                    div()
                        .id(("browse-identities", index))
                        .type_style(Type::Small)
                        .text_color(rgb(ACCENT))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.busy {
                                return;
                            }
                            if let Some(draft) = &mut this.logins.draft {
                                draft.tools.identity_open =
                                    if expanded { None } else { Some(index) };
                                cx.notify();
                            }
                        }))
                        .child(if expanded {
                            "Show matches"
                        } else {
                            "Browse all"
                        }),
                ),
        );
        body.child(
            div()
                .id(("identity-results", index))
                .max_h(px(180.))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(px(space::XS))
                .when(candidates.is_empty(), |s| {
                    s.child(
                        div()
                            .type_style(Type::Caption)
                            .text_color(rgb(MUTED))
                            .child("No saved match. Use the value you type."),
                    )
                })
                .children(
                    candidates
                        .into_iter()
                        .take(if expanded { usize::MAX } else { 3 })
                        .map(|(candidate, identity)| {
                            let input = input.clone();
                            div()
                                .id(("identity-choice", candidate))
                                .rounded(px(radius::STANDARD))
                                .p(px(space::SM))
                                .bg(rgb(BG))
                                .hover(|s| s.bg(rgb(HOVER)))
                                .cursor_pointer()
                                .flex()
                                .flex_col()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if this.busy {
                                        return;
                                    }
                                    if let Some(draft) = &mut this.logins.draft {
                                        let value =
                                            &draft.tools.suggestions.identities[candidate].value;
                                        input.update(cx, |i, cx| i.set_text(value, cx));
                                        draft.tools.identity_open = None;
                                        window.focus(&input.focus_handle(cx));
                                        cx.notify();
                                    }
                                }))
                                .child(
                                    div()
                                        .type_style(Type::Small)
                                        .font_family(font::MONO)
                                        .text_ellipsis()
                                        .child(identity.value.to_string()),
                                )
                                .child(
                                    div()
                                        .type_style(Type::Caption)
                                        .text_color(rgb(MUTED))
                                        .child(format!(
                                            "Used in {} {}",
                                            identity.uses,
                                            if identity.uses == 1 {
                                                "login"
                                            } else {
                                                "logins"
                                            }
                                        )),
                                )
                        }),
                ),
        )
    }
    pub(super) fn tag_suggestions(
        &self,
        index: usize,
        input: &Entity<TextInput>,
        cx: &mut Context<Self>,
    ) -> Div {
        let tools = &self.logins.draft.as_ref().unwrap().tools;
        let selected: Vec<_> = input
            .read(cx)
            .content
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        div()
            .flex()
            .flex_col()
            .gap(px(space::SM))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(space::XS))
                    .children(selected.iter().map(|tag| tag_chip((*tag).to_string()))),
            )
            .when(!tools.suggestions.tags.is_empty(), |s| {
                s.child(
                    div()
                        .type_style(Type::Caption)
                        .text_color(rgb(MUTED))
                        .child("Add an existing tag"),
                )
            })
            .child(
                div()
                    .id(("tag-suggestions", index))
                    .max_h(px(120.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_wrap()
                    .gap(px(space::XS))
                    .children(
                        tools
                            .suggestions
                            .tags
                            .iter()
                            .enumerate()
                            .filter(|(_, t)| !selected.contains(&t.as_str()))
                            .map(|(n, tag)| {
                                let input = input.clone();
                                let tag = tag.clone();
                                let chip = tag_chip(tag.clone());
                                div()
                                    .id(("suggested-tag", n))
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if this.busy {
                                            return;
                                        }
                                        let mut value =
                                            input.read(cx).content.trim_end().to_string();
                                        if !value.lines().any(|s| s.trim() == tag) {
                                            if !value.is_empty() {
                                                value.push('\n');
                                            }
                                            value.push_str(&tag);
                                            input.update(cx, |i, cx| i.set_text(&value, cx));
                                        }
                                    }))
                                    .child(chip)
                            }),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_the_latest_recipe_request_can_publish() {
        let a = PasswordOptions::default();
        let b = PasswordOptions { length: 32, ..a };
        let mut requests = PreviewRequests::default();
        let first = requests.request(Some(a), false).unwrap();
        assert!(requests.accepts(first, a));
        assert!(requests.request(Some(a), false).is_none());
        let second = requests.request(Some(b), false).unwrap();
        let third = requests.request(Some(a), false).unwrap();
        assert!(!requests.accepts(first, a));
        assert!(!requests.accepts(second, b));
        assert!(requests.accepts(third, a));
        requests.request(None, false).unwrap();
        assert!(!requests.accepts(third, a));
        let fourth = requests.request(Some(a), false).unwrap();
        assert!(requests.accepts(fourth, a));
        let variation = requests.request(Some(a), true).unwrap();
        assert!(!requests.accepts(fourth, a));
        assert!(requests.accepts(variation, a));
    }
    #[test]
    fn candidate_requires_the_same_field_and_recipe() {
        let options = PasswordOptions::default();
        let generated = GeneratedPassword {
            index: 1,
            options,
            value: zeroize::Zeroizing::new("synthetic-only".into()),
        };
        assert!(generated.matches(1, options));
        assert!(!generated.matches(2, options));
        assert!(!generated.matches(
            1,
            PasswordOptions {
                length: 32,
                ..options
            },
        ));
        assert!(!generated.matches(
            1,
            PasswordOptions {
                symbols: false,
                ..options
            },
        ));
    }
}
