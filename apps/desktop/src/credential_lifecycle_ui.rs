//! Reversible credential removal with an explicit, revision-bound confirmation.
use super::logins_ui::LoginScope;
use super::*;

pub(super) struct DeleteConfirmation {
    item: u64,
    revision: i64,
    title: String,
    cancel: FocusHandle,
    confirm: FocusHandle,
    error: Option<String>,
}
impl MeApp {
    pub(super) fn request_login_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.app_ready() || self.busy || self.login_edit_guard(cx) {
            return;
        }
        let Some(details) = &self.logins.details else {
            return;
        };
        let confirmation = DeleteConfirmation {
            item: details.credential.item_id,
            revision: details.revision,
            title: details.credential.title.clone(),
            cancel: cx.focus_handle(),
            confirm: cx.focus_handle(),
            error: None,
        };
        window.focus(&confirmation.cancel);
        self.logins.revealed.clear();
        self.logins.deleting = Some(confirmation);
        cx.notify();
    }
    pub(super) fn cancel_login_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let reload = self
            .logins
            .deleting
            .take()
            .is_some_and(|dialog| dialog.error.is_some());
        if reload && let Some(item) = self.logins.selected {
            self.select_login(item, cx);
        }
        window.focus(&self.login_focus);
        cx.notify();
    }
    fn deletion_tab(&self, window: &mut Window) {
        if let Some(dialog) = &self.logins.deleting {
            window.focus(if dialog.cancel.is_focused(window) {
                &dialog.confirm
            } else {
                &dialog.cancel
            });
        }
    }
    fn confirm_login_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = &self.logins.deleting else {
            return;
        };
        self.change_login_deletion(dialog.item, dialog.revision, true, window, cx);
    }
    fn restore_selected_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self
            .logins
            .items
            .iter()
            .find(|item| Some(item.id) == self.logins.selected && item.deleted_at.is_some())
        else {
            return;
        };
        self.change_login_deletion(item.id, item.revision, false, window, cx);
    }
    fn change_login_deletion(
        &mut self,
        item: u64,
        revision: i64,
        deleted: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy || !self.app_ready() {
            return;
        }
        self.busy = true;
        self.logins.error = None;
        if let Some(dialog) = &mut self.logins.deleting {
            dialog.error = None;
        }
        let session = self.session.clone();
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            let mut session = session.lock().map_err(|_| me_core::Error::Format)?;
            let vault = session.as_mut().ok_or(me_core::Error::Format)?;
            if deleted {
                vault.delete_login(item, revision)
            } else {
                vault.restore_login(item, revision)
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation {
                    return;
                }
                this.busy = false;
                match result {
                    Ok(()) => {
                        this.logins.deleting = None;
                        this.logins.revealed.clear();
                        this.logins.reveal_tokens.clear();
                        this.clear_owned_clipboard(cx);
                        this.logins.details = None;
                        this.logins.detail_request += 1;
                        this.logins.detail_loading = false;
                        this.logins.scope = if deleted {
                            LoginScope::Deleted
                        } else {
                            LoginScope::All
                        };
                        this.logins.items.clear();
                        this.logins.visible.clear();
                        this.logins.selected = Some(item);
                        this.login_search
                            .update(cx, |input, cx| input.set_text("", cx));
                        if !deleted {
                            this.select_login(item, cx);
                        }
                        this.refresh_logins(cx);
                        this.refresh_search(cx);
                        this.refresh_knowledge(cx);
                        window.focus(&this.login_focus);
                    }
                    Err(error) => {
                        if let Some(dialog) = &mut this.logins.deleting {
                            dialog.error = Some(error.to_string());
                        } else {
                            this.logins.error = Some(error.to_string());
                            this.refresh_logins(cx);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn login_delete_modal(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let dialog = self.logins.deleting.as_ref().unwrap();
        self.overlay(cx)
            .p(px(space::LG))
            .on_drop(|_: &ExternalPaths, _, cx| cx.stop_propagation())
            .child(modal_panel(480., self.motion_enabled())
                .id("delete-credential-modal")
                .max_h(window.viewport_size().height - px(space::SECTION))
                // Focused buttons activate once on native key-up; Enter is never a global submit.
                .on_action(|_: &Confirm, _, cx| cx.stop_propagation())
                .on_action(cx.listener(|this, _: &Dismiss, window, cx| {
                    cx.stop_propagation(); this.cancel_login_delete(window, cx);
                }))
                .on_action(cx.listener(|this, _: &NextField, window, cx| {
                    cx.stop_propagation(); this.deletion_tab(window);
                }))
                .on_action(cx.listener(|this, _: &PreviousField, window, cx| {
                    cx.stop_propagation(); this.deletion_tab(window);
                }))
                .child(div().flex_shrink_0().flex().items_center().gap(px(space::MD))
                    .child(icon(Icon::Honeycomb, IconSize::Large, ACCENT))
                    .child(heading("Move to Deleted?")))
                .child(div().id("delete-credential-body").min_h_0().overflow_y_scroll()
                    .flex().flex_col().gap(px(space::LG))
                    .child(div().p(px(space::LG)).bg(rgb(BG)).rounded(px(radius::STANDARD))
                        .type_style(Type::Label).whitespace_normal().child(dialog.title.clone()))
                    .child(div().type_style(Type::Body).text_color(rgb(MUTED))
                        .child("This item will leave your logins, search and suggestions. You can restore it from Deleted at any time."))
                    .child(div().type_style(Type::Small).text_color(rgb(MUTED))
                        .child("All fields, attachments and password history stay encrypted in your vault. Nothing is permanently erased."))
                    .when_some(dialog.error.clone(), |s, error| s.child(div().type_style(Type::Small).text_color(rgb(DANGER)).child(error))))
                .child(div().flex_shrink_0().flex().justify_end().gap(px(space::SM))
                    .child(secondary_action().id("cancel-delete-credential").track_focus(&dialog.cancel)
                        .focus(|s| s.border_color(rgb(FOCUS)))
                        .on_click(cx.listener(|this, _, window, cx| this.cancel_login_delete(window, cx)))
                        .child("Cancel"))
                    .child(primary_action().id("confirm-delete-credential").track_focus(&dialog.confirm)
                        .bg(rgb(DANGER)).focus(|s| s.border_1().border_color(rgb(FOCUS)))
                        .on_click(cx.listener(|this, _, window, cx| this.confirm_login_delete(window, cx)))
                        .child(if self.busy { "Moving…" } else { "Move to Deleted" }))))
    }
    pub(super) fn deleted_login_detail(
        &self,
        phase: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self
            .logins
            .items
            .iter()
            .find(|item| Some(item.id) == self.logins.selected && item.deleted_at.is_some());
        div().w_full().max_w(px(layout::CONTENT_WIDTH)).mx_auto().flex().flex_col().gap(px(space::XXL))
            .child(motion::login_identity(phase, None))
            .child(eyebrow("Deleted · recoverable"))
            .child(heading(selected.map(|i| i.title.clone()).unwrap_or_else(|| if self.logins.loading { "Loading…" } else if !self.logins.items.is_empty() { "No matching deleted items" } else { "Nothing in Deleted" }.into())).whitespace_normal())
            .child(div().type_style(Type::Body).text_color(rgb(MUTED)).child(if selected.is_some() {
                "Restore this item to use it again. Its fields, attachments and full password history are kept in your vault."
            } else if !self.logins.items.is_empty() { "Try a different name or vault. Your deleted items are still recoverable." } else { "Deleted items stay here until you restore them. There is no automatic expiry." }))
            .when_some(selected, |s, item| s
                .child(div().type_style(Type::Caption).font_family(font::MONO).text_color(rgb(MUTED))
                    .child(format!("Deleted {}", item.deleted_at.as_deref().unwrap_or_default().replace('T', " ").replace('Z', " UTC"))))
                .child(div().flex().child(primary_action().id("restore-credential")
                    .on_click(cx.listener(|this, _, window, cx| {
                        window.focus(&this.login_focus); this.restore_selected_login(window, cx)
                    })).child(if self.busy { "Restoring…" } else { "Restore item" }))))
            .when_some(self.logins.error.clone(), |s, error| s.child(div().type_style(Type::Small).text_color(rgb(DANGER)).child(error)))
    }
}
