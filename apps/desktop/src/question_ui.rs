use super::*;

impl MeApp {
    pub(super) fn answer_question(
        &mut self,
        item: u64,
        question: String,
        answer: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if !self.app_ready() || self.busy || (self.active_imports.contains_key(&item)) {
            return;
        }
        self.busy = true;
        self.question_error = None;
        let session = self.session.clone();
        let generation = self.generation;
        let error_question = question.clone();
        let task = cx.background_executor().spawn(async move {
            let mut guard = session.lock().map_err(|_| me_core::Error::Format)?;
            let vault = guard
                .as_mut()
                .ok_or(me_core::Error::Validation("Vault locked."))?;
            vault.answer_review_question(item, &question, answer.as_deref())?;
            me_diagnostics::record(
                "review.answered",
                &[
                    me_diagnostics::Field::Count("item", item),
                    me_diagnostics::Field::Flag("confirmed", answer.is_some()),
                ],
            );
            Ok::<_, me_core::Error>((
                vault.collection("", false)?,
                vault.review_questions(item)?,
                vault.evaluation_warning(item)?,
            ))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.busy = false;
                match result {
                    Ok((collection, questions, warning)) => {
                        if this.search.read(cx).content.is_empty() && !this.pinned_only {
                            this.collection = collection;
                        }
                        if this.document_open == Some(item) {
                            this.questions = questions;
                            this.question_edit = None;
                            this.document_warning = warning.clone();
                        }
                        if let Some(warning) = warning {
                            this.ai_warning = Some((item, warning));
                        } else if this.ai_warning.as_ref().is_some_and(|(id, _)| *id == item) {
                            this.ai_warning = None;
                        }
                        this.ai_message = None;
                        this.notice = Some("Saved. Conflicting values are kept for review.".into());
                    }
                    Err(error) => {
                        this.question_error = Some((error_question, ai_ui::core_error(error)));
                    }
                }
                this.refresh_search(cx);
                this.kick_auto_queue(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn question_card(
        &self,
        question: &me_core::ReviewQuestion,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let item = self.document_open.unwrap_or_default();
        let question = question.clone();
        let key = question.id.clone();
        let accept_key = key.clone();
        let edit_key = key.clone();
        let dismiss_key = key.clone();
        let accept_value = question.value.clone();
        let edit_value = question.value.clone();
        let editing = self
            .question_edit
            .as_ref()
            .filter(|(id, _)| id == &key)
            .map(|(_, input)| input.clone());
        let enabled = !self.busy && !(self.active_imports.contains_key(&item));
        div()
            .id(gpui::ElementId::Name(format!("question-{key}").into()))
            .flex_shrink_0()
            .p(px(space::LG))
            .rounded(px(radius::STANDARD))
            .bg(rgb(WARNING_SURFACE))
            .flex()
            .flex_col()
            .gap(px(space::SM))
            .child(
                div()
                    .type_style(Type::Caption)
                    .text_color(rgb(WARNING))
                    .child("NEEDS CONFIRMATION"),
            )
            .child(
                div()
                    .type_style(Type::Label)
                    .font_weight(font::EMPHASIS)
                    .child(format!("{}: {}", question.label, question.value)),
            )
            .child(
                div()
                    .type_style(Type::Small)
                    .child("Is this correct and yours?"),
            )
            .child(
                div()
                    .type_style(Type::Small)
                    .text_color(rgb(MUTED))
                    .child(question.reason),
            )
            .when(!question.existing_values.is_empty(), |s| {
                s.child(div().type_style(Type::Small).child(format!(
                    "Already saved: {}",
                    question.existing_values.join(" · ")
                )))
            })
            .when(!question.claimed_subject.is_empty(), |s| {
                s.child(
                    div()
                        .type_style(Type::Caption)
                        .text_color(rgb(MUTED))
                        .child(format!(
                            "Suggested person (unverified): {}",
                            question.claimed_subject
                        )),
                )
            })
            .when(!question.claimed_quote.is_empty(), |s| {
                s.child(
                    div()
                        .type_style(Type::Caption)
                        .text_color(rgb(MUTED))
                        .child(format!(
                            "Suggested quotation (unverified): {}",
                            question.claimed_quote
                        )),
                )
            })
            .when_some(question.source_excerpt, |s, text| {
                s.child(
                    div()
                        .p(px(space::SM))
                        .rounded(px(radius::STANDARD))
                        .bg(rgb(SURFACE))
                        .type_style(Type::Caption)
                        .child(format!(
                            "Document text · {}: {}{}",
                            question.location_label.unwrap_or_else(|| "Source".into()),
                            text.chars().take(700).collect::<String>(),
                            if text.chars().count() > 700 {
                                " …"
                            } else {
                                ""
                            }
                        )),
                )
            })
            .child(
                div()
                    .type_style(Type::Caption)
                    .text_color(rgb(MUTED))
                    .child("Approval saves this as your own statement."),
            )
            .when_some(editing.clone(), |s, input| {
                let focus_input = input.clone();
                s.child(div().type_style(Type::Small).child("Correct value"))
                    .child(
                        div()
                            .id(gpui::ElementId::Name(format!("answer-{key}").into()))
                            .h(px(layout::CONTROL_LARGE))
                            .flex_shrink_0()
                            .px(px(space::MD))
                            .rounded(px(radius::STANDARD))
                            .border_1()
                            .border_color(rgb(LINE))
                            .bg(rgb(SURFACE))
                            .flex()
                            .items_center()
                            .on_click(move |_, window, cx| {
                                window.focus(&focus_input.focus_handle(cx))
                            })
                            .child(input),
                    )
            })
            .when(enabled, |s| {
                s.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(space::MD))
                        .child(
                            div()
                                .id(gpui::ElementId::Name(format!("confirm-{key}").into()))
                                .type_style(Type::Small)
                                .text_color(rgb(ACCENT))
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    let value = this
                                        .question_edit
                                        .as_ref()
                                        .filter(|(id, _)| id == &accept_key)
                                        .map(|(_, input)| input.read(cx).content.to_string())
                                        .unwrap_or_else(|| accept_value.clone());
                                    window.focus(&this.focus);
                                    this.answer_question(item, accept_key.clone(), Some(value), cx);
                                }))
                                .child(if editing.is_some() {
                                    "Save correction"
                                } else {
                                    "Approve"
                                }),
                        )
                        .when(editing.is_none(), |s| {
                            s.child(
                                div()
                                    .id(gpui::ElementId::Name(format!("edit-{key}").into()))
                                    .type_style(Type::Small)
                                    .text_color(rgb(ACCENT))
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        let input = cx.new(|cx| {
                                            let mut input = TextInput::new("Correct value", cx);
                                            input.set_text(&edit_value, cx);
                                            input
                                        });
                                        window.focus(&input.focus_handle(cx));
                                        this.question_edit = Some((edit_key.clone(), input));
                                        this.question_error = None;
                                        cx.notify();
                                    }))
                                    .child("Edit"),
                            )
                        })
                        .child(
                            div()
                                .id(gpui::ElementId::Name(format!("dismiss-{key}").into()))
                                .type_style(Type::Small)
                                .text_color(rgb(MUTED))
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    window.focus(&this.focus);
                                    this.answer_question(item, dismiss_key.clone(), None, cx);
                                }))
                                .child("Not mine"),
                        ),
                )
            })
            .when_some(
                self.question_error
                    .as_ref()
                    .filter(|(id, _)| id == &key)
                    .map(|(_, error)| error.clone()),
                |s, error| {
                    s.child(
                        div()
                            .type_style(Type::Small)
                            .text_color(rgb(DANGER))
                            .child(error),
                    )
                },
            )
    }
}
