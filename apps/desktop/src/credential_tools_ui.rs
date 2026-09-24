//! Local editor tools. Candidates and generated secrets never leave the vault/device.
use super::*;
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
    _length_subscription: gpui::Subscription,
}
impl CredentialTools {
    pub fn new(cx: &mut Context<MeApp>) -> Self {
        let length = cx.new(|cx| {
            let mut input = TextInput::new("8–128", cx);
            input.set_text("20", cx);
            input
        });
        let subscription = cx.observe(&length, |_, _, cx| cx.notify());
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
    pub(super) fn toggle_password_generator(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if let Some(draft) = &mut self.logins.draft {
            draft.tools.generator_open = if draft.tools.generator_open == Some(index) {
                None
            } else {
                Some(index)
            };
            cx.notify();
        }
    }
    fn generate_login_password(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(draft) = &mut self.logins.draft else {
            return;
        };
        if draft.tools.generating {
            return;
        }
        let Some((_, input)) = draft.fields.iter().find(|(i, _)| *i == index) else {
            return;
        };
        let Ok(length) = draft.tools.length.read(cx).content.trim().parse::<usize>() else {
            draft.tools.password_error = Some("Choose a password length between 8 and 128.".into());
            cx.notify();
            return;
        };
        let options = PasswordOptions {
            length,
            ..draft.tools.options
        };
        let input = input.clone();
        let previous = zeroize::Zeroizing::new(input.read(cx).content.to_string());
        let draft_id = draft.title.entity_id();
        draft.tools.generating = true;
        draft.tools.password_error = None;
        self.logins.error = None;
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
                if draft.title.entity_id() != draft_id {
                    return;
                }
                draft.tools.generating = false;
                if this.busy || input.read(cx).content.as_ref() != previous.as_str() {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(secret) => {
                        input.update(cx, |i, cx| {
                            i.set_concealed(true, cx);
                            i.set_text(&secret, cx);
                        });
                        this.logins.revealed.remove(&index);
                        this.logins.notice = Some(format!(
                            "Generated locally · {length} characters. Save after reviewing."
                        ));
                    }
                    Err(error) => draft.tools.password_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn password_generator(&self, index: usize, cx: &mut Context<Self>) -> Div {
        let tools = &self.logins.draft.as_ref().unwrap().tools;
        div()
            .flex()
            .flex_col()
            .gap(px(space::MD))
            .pt(px(space::MD))
            .border_t_1()
            .border_color(rgb(LINE))
            .child(
                div()
                    .type_style(Type::Small)
                    .text_color(rgb(MUTED))
                    .child("Password options"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::SM))
                    .child(
                        div()
                            .flex_1()
                            .type_style(Type::Small)
                            .child("Length · 8–128"),
                    )
                    .child(div().w(px(96.)).child(self.login_input_frame(
                        &tools.length,
                        false,
                        cx,
                    ))),
            )
            .child(
                div().flex().flex_wrap().gap(px(space::SM)).children(
                    [
                        (0, tools.options.uppercase, "Uppercase A–Z"),
                        (1, tools.options.lowercase, "Lowercase a–z"),
                        (2, tools.options.digits, "Numbers 0–9"),
                        (3, tools.options.symbols, "Symbols !@#"),
                        (4, tools.options.exclude_ambiguous, "Avoid 0 O 1 I l"),
                    ]
                    .into_iter()
                    .map(|(key, checked, label)| {
                        div()
                            .id(("password-option", key as usize))
                            .min_w(px(144.))
                            .flex_1()
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if this.busy {
                                    return;
                                }
                                if let Some(draft) = &mut this.logins.draft {
                                    let o = &mut draft.tools.options;
                                    let value = match key {
                                        0 => &mut o.uppercase,
                                        1 => &mut o.lowercase,
                                        2 => &mut o.digits,
                                        3 => &mut o.symbols,
                                        _ => &mut o.exclude_ambiguous,
                                    };
                                    *value = !*value;
                                    cx.notify();
                                }
                            }))
                            .child(checkbox(checked, label))
                    }),
                ),
            )
            .child(
                div()
                    .type_style(Type::Caption)
                    .text_color(rgb(MUTED))
                    .child(
                        "Every selected character type is included. Generated only on this device.",
                    ),
            )
            .when_some(tools.password_error.clone(), |s, error| {
                s.child(
                    div()
                        .type_style(Type::Small)
                        .text_color(rgb(DANGER))
                        .child(error),
                )
            })
            .child(
                primary_action()
                    .id(("apply-password-options", index))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.generate_login_password(index, cx)),
                    )
                    .child(if tools.generating {
                        "Generating…"
                    } else {
                        "Generate password"
                    }),
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
