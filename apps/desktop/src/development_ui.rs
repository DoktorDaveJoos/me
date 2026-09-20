use super::*;

#[derive(Default)]
pub(super) struct DevelopmentState {
    pub confirming: bool,
    pub wiping: bool,
}

impl MeApp {
    pub(super) fn wipe_data(&mut self, cx: &mut Context<Self>) {
        if !self.app_ready() || self.busy || !self.development.confirming {
            return;
        }
        let old = self.detach_vault(cx);
        self.development.wiping = true;
        let session = self.session.clone();
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            // Take the vault out of the old session before resetting. Any late
            // worker/checkpoint can only see None, even when local IDs are reused.
            let mut vault = old
                .lock()
                .map_err(|_| me_core::Error::Format)?
                .take()
                .ok_or(me_core::Error::Validation("Vault locked."))?;
            let result = vault.wipe_data_for_development();
            let collection = vault.collection("", false)?;
            let settings = vault.settings()?;
            *session.lock().map_err(|_| me_core::Error::Format)? = Some(vault);
            Ok::<_, me_core::Error>((result, collection, settings))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.development.wiping = false;
                this.busy = false;
                match result {
                    Ok((result, collection, settings)) => {
                        this.unlocked = true;
                        this.show_settings = true;
                        this.show_onepassword = false;
                        this.collection = collection;
                        this.settings = settings;
                        match result {
                            Ok(()) => {
                                this.notice =
                                    Some("Data wiped. You can upload your files again.".into())
                            }
                            Err(error) => this.error = Some(error.to_string()),
                        }
                        this.refresh_search(cx);
                        this.refresh_imports(cx);
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn development_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div().w_full().min_w_0().max_w(px(layout::CONTENT_WIDTH)).flex_shrink_0().pt(px(space::LG)).border_t_1().border_color(rgb(LINE))
            .flex().flex_col().gap(px(space::MD))
            .child(eyebrow("DEVELOPMENT"))
            .child(div().w_full().min_w_0().max_w_full().flex_shrink_0().whitespace_normal().type_style(Type::Small).text_color(rgb(MUTED))
                .child("Clear this vault’s files, notes, credentials, extracted data and import history. Your password, preferences and ChatGPT connection are kept."))
            .when(!self.development.confirming, |s| s.child(
                primary_action().flex_shrink_0().id("wipe-data").bg(rgb(DANGER)).hover(|s| s.bg(rgb(INK)))
                    .when(self.busy, |s| s.opacity(0.5))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.busy { return; }
                        this.development.confirming = true;
                        this.error = None;
                        this.notice = None;
                        cx.notify();
                    })).child("Wipe data")))
            .when(self.development.confirming, |s| s.child(
                div().w_full().flex_shrink_0().p(px(space::LG)).rounded(px(radius::STANDARD)).bg(rgb(DANGER_SURFACE))
                    .flex().flex_col().gap(px(space::MD))
                    .child(div().type_style(Type::Label).text_color(rgb(DANGER)).child("Wipe all local data?"))
                    .child(div().w_full().min_w_0().max_w_full().flex_shrink_0().whitespace_normal().type_style(Type::Small).text_color(rgb(DANGER))
                        .child("This cannot be undone. Active work will stop. Original files and external backups are untouched."))
                    .child(div().flex().items_center().justify_end().gap(px(space::LG))
                        .child(primary_action().flex_shrink_0().id("cancel-wipe").bg(rgb(SURFACE)).text_color(rgb(INK))
                            .hover(|s| s.bg(rgb(HOVER)))
                            .on_click(cx.listener(|this, _, _, cx| { this.development.confirming = false; cx.notify(); }))
                            .child("Cancel"))
                        .child(primary_action().flex_shrink_0().id("confirm-wipe").bg(rgb(DANGER)).hover(|s| s.bg(rgb(INK)))
                            .on_click(cx.listener(|this, _, _, cx| this.wipe_data(cx)))
                            .child("Wipe data")))))
    }

    pub(super) fn wipe_progress(&self) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(BG))
            .font_family(font::SANS)
            .text_color(rgb(INK))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(space::LG))
            .child(heading("Wiping data…"))
            .child(
                div()
                    .type_style(Type::Body)
                    .text_color(rgb(MUTED))
                    .child("Stopping active work and clearing this vault."),
            )
    }
}
