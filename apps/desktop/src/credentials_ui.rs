use super::*;
use gpui::Div;
use me_diagnostics::{Field as F, record};

fn onepassword_failure(stage: &'static str, error: &me_core::Error) {
    let code = match error {
        me_core::Error::Io(_) => "io",
        me_core::Error::Database(_) => "database",
        me_core::Error::Authentication => "authentication",
        me_core::Error::Format => "format",
        me_core::Error::InUse => "in_use",
        me_core::Error::Validation(_) => "validation",
    };
    let mut fields = vec![F::Label("stage", stage), F::Label("code", code)];
    // Validation messages are static and contain no filenames or credential data.
    if let me_core::Error::Validation(reason) = error {
        fields.push(F::Label("reason", reason));
    }
    record("onepassword.failed", &fields);
}

/// The 1Password import dialog shows one step at a time. Preview and result
/// data stay in `onepassword_preview` / `onepassword_result`.
#[derive(Default)]
pub(super) enum OnePasswordStep {
    #[default]
    Choose,
    Checking,
    Review,
    /// Keeps the confirmed counts visible while the vault transaction runs.
    Importing(me_core::OnePasswordSummary),
    Done,
}

const IMPORT_STEPS: &[&str] = &["Export", "Review", "Import"];

impl MeApp {
    pub(super) fn clear_credentials(&mut self, cx: &mut Context<Self>) {
        self.credential_generation += 1;
        self.onepassword_step = OnePasswordStep::Choose;
        self.onepassword_result = None;
        self.credential_revealed.clear();
        let preview = self.onepassword_preview.take();
        let credential = self.credential.take();
        cx.background_executor()
            .spawn(async move {
                drop(preview);
                drop(credential);
            })
            .detach();
    }

    pub(super) fn pick_onepassword(&mut self, cx: &mut Context<Self>) {
        if !self.app_ready() || self.busy {
            return;
        }
        self.error = None;
        self.notice = None;
        let generation = self.credential_generation;
        record("onepassword.picker.opened", &[]);
        let task = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose 1PUX".into()),
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.credential_generation != generation || !this.app_ready() {
                    return;
                }
                match result {
                    Ok(Ok(Some(paths))) if paths.len() == 1 => {
                        this.prepare_onepassword(paths[0].clone(), cx)
                    }
                    Ok(Ok(None)) => record("onepassword.picker.cancelled", &[]),
                    _ => {
                        record("onepassword.failed", &[F::Label("stage", "picker")]);
                        this.error = Some("Couldn't open the file picker.".into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    pub(super) fn prepare_onepassword(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !self.app_ready() || self.busy {
            return;
        }
        self.clear_credentials(cx);
        self.onepassword_step = OnePasswordStep::Checking;
        self.busy = true;
        self.error = None;
        let session = self.session.clone();
        let generation = self.credential_generation;
        let task = cx.background_executor().spawn(async move {
            record("onepassword.read.started", &[]);
            let import = me_core::OnePasswordImport::read(&path).inspect_err(|error| {
                onepassword_failure("read", error);
            })?;
            let summary = (|| {
                let session = session.lock().map_err(|_| me_core::Error::Format)?;
                let vault = session
                    .as_ref()
                    .ok_or(me_core::Error::Validation("Vault locked."))?;
                vault.preview_onepassword(&import)
            })()
            .inspect_err(|error| onepassword_failure("preview", error))?;
            record(
                "onepassword.preview.ready",
                &[
                    F::Count("items", summary.total as u64),
                    F::Count("new", summary.new as u64),
                    F::Count("duplicates", summary.duplicates as u64),
                    F::Count("changed", summary.changed as u64),
                    F::Count("files", summary.files as u64),
                ],
            );
            Ok::<_, me_core::Error>((import, summary))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.credential_generation != generation || !this.app_ready() {
                    return;
                }
                this.busy = false;
                match result {
                    Ok(preview) => {
                        this.onepassword_preview = Some(preview);
                        this.onepassword_step = OnePasswordStep::Review;
                    }
                    Err(e) => {
                        this.error = Some(e.to_string());
                        this.onepassword_step = OnePasswordStep::Choose;
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn commit_onepassword(&mut self, cx: &mut Context<Self>) {
        if !self.app_ready() || self.busy {
            return;
        }
        let Some((import, summary)) = self.onepassword_preview.take() else {
            return;
        };
        self.onepassword_step = OnePasswordStep::Importing(summary.clone());
        self.busy = true;
        self.error = None;
        self.notice = None;
        let session = self.session.clone();
        let generation = self.credential_generation;
        let task = cx.background_executor().spawn(async move {
            record("onepassword.import.started", &[]);
            let result = (|| {
                let mut session = session.lock().map_err(|_| me_core::Error::Format)?;
                let vault = session
                    .as_mut()
                    .ok_or(me_core::Error::Validation("Vault locked."))?;
                vault.import_onepassword(&import)
            })();
            match &result {
                Ok(summary) => record(
                    "onepassword.import.completed",
                    &[
                        F::Count("imported", summary.to_import() as u64),
                        F::Count("duplicates", summary.duplicates as u64),
                        F::Count("changed", summary.changed as u64),
                    ],
                ),
                Err(error) => onepassword_failure("import", error),
            }
            (import, result)
        });
        cx.spawn(async move |this, cx| {
            let (import, result) = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.credential_generation != generation || !this.app_ready() {
                    return;
                }
                this.busy = false;
                match result {
                    Ok(result) => {
                        this.onepassword_result = Some(result);
                        this.onepassword_step = OnePasswordStep::Done;
                        if this.page == Page::Logins {
                            this.refresh_logins(cx);
                        }
                        this.refresh_search(cx);
                    }
                    Err(error) => {
                        this.error = Some(error.to_string());
                        this.onepassword_preview = Some((import, summary));
                        this.onepassword_step = OnePasswordStep::Review;
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Opens the import dialog on Logins at its first step.
    pub(super) fn open_onepassword_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.app_ready() || self.busy || self.login_edit_guard(cx) {
            return;
        }
        if self.page != Page::Logins || self.show_settings {
            self.navigate(Page::Logins, window, cx);
        }
        self.clear_credentials(cx);
        self.show_onepassword = true;
        self.error = None;
        self.notice = None;
        cx.notify();
    }

    fn close_onepassword_import(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.show_onepassword = false;
        self.error = None;
        self.clear_credentials(cx);
        self.refresh_logins(cx);
        cx.notify();
    }

    fn restart_onepassword_import(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.error = None;
        self.clear_credentials(cx);
        cx.notify();
    }

    pub(super) fn onepassword_modal(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let (rail, title, subtitle) = match &self.onepassword_step {
            OnePasswordStep::Choose => (
                0,
                "Import from 1Password",
                "Bring your 1Password entries into your encrypted ME. vault.",
            ),
            OnePasswordStep::Checking => (
                1,
                "Checking your export",
                "Reading the file on this device. Nothing is saved yet.",
            ),
            OnePasswordStep::Review => (
                1,
                "Review your import",
                "This is exactly what will be added. Nothing is saved until you confirm.",
            ),
            OnePasswordStep::Importing(_) => {
                (1, "Review your import", "Saving to your encrypted vault…")
            }
            OnePasswordStep::Done => (
                3,
                "Import complete",
                "Your entries are saved in your encrypted vault.",
            ),
        };
        let body = match &self.onepassword_step {
            OnePasswordStep::Choose => self.onepassword_choose_body(),
            OnePasswordStep::Checking => self.onepassword_checking_body(),
            OnePasswordStep::Review => match self.onepassword_preview.as_ref() {
                Some((_, summary)) => self.onepassword_review_body(summary),
                None => div(),
            },
            OnePasswordStep::Importing(summary) => self.onepassword_review_body(summary),
            OnePasswordStep::Done => match self.onepassword_result.as_ref() {
                Some(result) => self.onepassword_done_body(result),
                None => div(),
            },
        };
        self.overlay(cx).p(px(space::LG)).child(
            modal_panel(600., self.motion_enabled())
                .id("onepassword-import-modal")
                .max_h(window.viewport_size().height - px(space::SECTION))
                .gap(px(space::XL))
                .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                    cx.stop_propagation();
                    let choosing = matches!(this.onepassword_step, OnePasswordStep::Choose);
                    match paths.paths() {
                        [path]
                            if choosing
                                && path
                                    .extension()
                                    .is_some_and(|e| e.eq_ignore_ascii_case("1pux")) =>
                        {
                            this.prepare_onepassword(path.clone(), cx)
                        }
                        _ if choosing => {
                            this.error = Some("Drop a single .1pux export from 1Password.".into());
                            cx.notify();
                        }
                        _ => {}
                    }
                }))
                .child(
                    div()
                        .flex_shrink_0()
                        .flex()
                        .flex_col()
                        .gap(px(space::XL))
                        .child(
                            div()
                                .flex()
                                .items_start()
                                .gap(px(space::MD))
                                .child(
                                    div()
                                        .size(px(layout::CONTROL_LARGE))
                                        .flex_shrink_0()
                                        .rounded(px(radius::STANDARD))
                                        .bg(rgb(HOVER))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(icon(Icon::Key, IconSize::Large, ACCENT)),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .gap(px(space::XS))
                                        .child(heading(title))
                                        .child(
                                            div()
                                                .whitespace_normal()
                                                .type_style(Type::Small)
                                                .text_color(rgb(MUTED))
                                                .child(subtitle),
                                        ),
                                )
                                .when(!self.busy, |s| {
                                    s.child(
                                        icon_action(
                                            "close-onepassword-import",
                                            Icon::Close,
                                            "Close",
                                        )
                                        .on_click(
                                            cx.listener(|this, _, _, cx| {
                                                this.close_onepassword_import(cx)
                                            }),
                                        ),
                                    )
                                }),
                        )
                        .child(step_rail(IMPORT_STEPS, rail)),
                )
                .child(
                    div()
                        .id(("onepassword-import-body", rail))
                        .min_h_0()
                        .overflow_y_scroll()
                        .child(body),
                )
                .child(self.onepassword_footer(cx)),
        )
    }

    fn onepassword_footer(&self, cx: &mut Context<Self>) -> Div {
        let footer = div()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(space::SM));
        let animated = self.motion_enabled();
        match &self.onepassword_step {
            OnePasswordStep::Choose => footer
                .child(
                    secondary_action()
                        .id("cancel-onepassword")
                        .hover(|s| s.bg(rgb(HOVER)))
                        .on_click(cx.listener(|this, _, _, cx| this.close_onepassword_import(cx)))
                        .child("Cancel"),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .type_style(Type::Caption)
                        .text_color(rgb(MUTED))
                        .child("or drop the file here"),
                )
                .child(
                    primary_action()
                        .id("pick-onepassword")
                        .hover(|s| s.bg(rgb(PRIMARY_HOVER)))
                        .on_click(cx.listener(|this, _, _, cx| this.pick_onepassword(cx)))
                        .child("Choose 1PUX file…")
                        .child(action_indicator(false, animated)),
                ),
            OnePasswordStep::Checking => footer.child(div().flex_1()).child(
                primary_action()
                    .id("checking-onepassword")
                    .cursor_default()
                    .child("Checking…")
                    .child(action_indicator(true, animated)),
            ),
            OnePasswordStep::Review => {
                let count = self
                    .onepassword_preview
                    .as_ref()
                    .map_or(0, |(_, summary)| summary.to_import());
                footer
                    .child(
                        secondary_action()
                            .id("back-onepassword")
                            .hover(|s| s.bg(rgb(HOVER)))
                            .on_click(
                                cx.listener(|this, _, _, cx| this.restart_onepassword_import(cx)),
                            )
                            .child("Choose another file"),
                    )
                    .child(div().flex_1())
                    .child(if count > 0 {
                        primary_action()
                            .id("confirm-onepassword")
                            .hover(|s| s.bg(rgb(PRIMARY_HOVER)))
                            .on_click(cx.listener(|this, _, _, cx| this.commit_onepassword(cx)))
                            .child(entries(count, "Import"))
                            .child(action_indicator(false, animated))
                    } else {
                        primary_action()
                            .id("close-empty-onepassword")
                            .hover(|s| s.bg(rgb(PRIMARY_HOVER)))
                            .on_click(
                                cx.listener(|this, _, _, cx| this.close_onepassword_import(cx)),
                            )
                            .child("Close")
                    })
            }
            OnePasswordStep::Importing(summary) => footer
                .child(
                    secondary_action()
                        .id("back-onepassword")
                        .cursor_default()
                        .opacity(0.5)
                        .child("Choose another file"),
                )
                .child(div().flex_1())
                .child(
                    primary_action()
                        .id("confirm-onepassword")
                        .cursor_default()
                        .child(entries(summary.to_import(), "Importing"))
                        .child(action_indicator(true, animated)),
                ),
            OnePasswordStep::Done => footer
                .child(
                    secondary_action()
                        .id("import-another-onepassword")
                        .hover(|s| s.bg(rgb(HOVER)))
                        .on_click(cx.listener(|this, _, _, cx| this.restart_onepassword_import(cx)))
                        .child("Import another export"),
                )
                .child(div().flex_1())
                .child(
                    primary_action()
                        .id("show-imported-credentials")
                        .hover(|s| s.bg(rgb(PRIMARY_HOVER)))
                        .on_click(cx.listener(|this, _, _, cx| this.close_onepassword_import(cx)))
                        .child("View logins")
                        .child(action_indicator(false, animated)),
                ),
        }
    }

    fn onepassword_error(&self) -> Option<Div> {
        self.error.clone().map(|error| {
            div()
                .p(px(space::LG))
                .rounded(px(radius::STANDARD))
                .bg(rgb(DANGER_SURFACE))
                .flex()
                .items_start()
                .gap(px(space::SM))
                .child(icon(Icon::Info, IconSize::Medium, DANGER))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .whitespace_normal()
                        .type_style(Type::Small)
                        .text_color(rgb(DANGER))
                        .child(error),
                )
        })
    }

    fn onepassword_choose_body(&self) -> Div {
        let how = [
            "Open 1Password 8 on this computer.",
            "Choose File → Export, then select the account.",
            "Pick the 1PUX format and save the file.",
        ];
        let included = [
            (
                Icon::Key,
                "Logins and passwords, including password history",
            ),
            (
                Icon::Text,
                "Secure notes, cards, identities and every other entry type",
            ),
            (
                Icon::Hash,
                "Custom fields, one-time password secrets and tags",
            ),
            (Icon::Attach, "Attachments, archived entries and favorites"),
        ];
        div()
            .flex()
            .flex_col()
            .gap(px(space::LG))
            .children(self.onepassword_error())
            .child(
                card()
                    .child(eyebrow("EXPORT FROM 1PASSWORD"))
                    .children(how.iter().enumerate().map(|(index, step)| {
                        div()
                            .flex()
                            .items_start()
                            .gap(px(space::MD))
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .type_style(Type::Body)
                                    .font_family(font::MONO)
                                    .text_color(rgb(ACCENT))
                                    .child(format!("0{}", index + 1)),
                            )
                            .child(div().flex_1().min_w_0().whitespace_normal().type_style(Type::Body).child(*step))
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::SM))
                    .child(eyebrow("WHAT COMES ACROSS"))
                    .children(included.into_iter().map(|(glyph, text)| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(space::MD))
                            .child(icon(glyph, IconSize::Medium, MUTED))
                            .child(div().flex_1().min_w_0().whitespace_normal().type_style(Type::Body).child(text))
                    })),
            )
            .child(
                div()
                    .whitespace_normal()
                    .type_style(Type::Small)
                    .text_color(rgb(MUTED))
                    .child("Passkeys aren’t part of 1Password exports. CSV and 1PIF files aren’t supported. Everything stays encrypted on this device and is never shared with AI."),
            )
    }

    fn onepassword_checking_body(&self) -> Div {
        card()
            .items_center()
            .py(px(space::SECTION))
            .child(motion::spinner(self.motion_enabled()))
            .child(div().type_style(Type::Label).child("Reading and validating the export"))
            .child(
                div()
                    .whitespace_normal()
                    .type_style(Type::Small)
                    .text_color(rgb(MUTED))
                    .child("Large exports can take a moment. Entries are compared with your vault to find duplicates."),
            )
    }

    fn onepassword_review_body(&self, summary: &me_core::OnePasswordSummary) -> Div {
        let adding = summary.to_import();
        let mut notes = Vec::new();
        if summary.archived > 0 {
            notes.push(if summary.archived == 1 {
                "1 archived entry keeps its Archived label.".into()
            } else {
                format!(
                    "{} archived entries keep their Archived label.",
                    summary.archived
                )
            });
        }
        if summary.files > 0 {
            notes.push(format!(
                "{} {} in the export are stored with the original.",
                summary.files,
                if summary.files == 1 { "file" } else { "files" }
            ));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(space::XL))
            .children(self.onepassword_error())
            .child(
                card()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(space::SM))
                            .child(div().type_style(Type::Title).child(adding.to_string()))
                            .child(div().type_style(Type::Label).text_color(rgb(MUTED)).child(if adding == 1 {
                                "entry will be added"
                            } else {
                                "entries will be added"
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(space::SM))
                            .child(stat("New", summary.new, "Not yet in ME."))
                            .child(stat("Changed", summary.changed, "Saved as another version."))
                            .child(stat("Already in ME", summary.duplicates, "Skipped.")),
                    ),
            )
            .when(summary.total == 0, |s| {
                s.child(div().type_style(Type::Body).text_color(rgb(MUTED)).child("This export contains no entries."))
            })
            .when(summary.total > 0 && adding == 0, |s| {
                s.child(
                    div()
                        .whitespace_normal()
                        .type_style(Type::Body)
                        .text_color(rgb(MUTED))
                        .child("Everything in this export is already in ME. There’s nothing new to import."),
                )
            })
            .when(!summary.categories.is_empty(), |s| {
                s.child(group_list("BY TYPE", &summary.categories, None))
            })
            .when(!summary.vaults.is_empty(), |s| {
                s.child(group_list("BY VAULT", &summary.vaults, Some(Icon::Folder)))
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::XS))
                    .children(notes.into_iter().map(|note| {
                        div().whitespace_normal().type_style(Type::Small).text_color(rgb(MUTED)).child(note)
                    }))
                    .child(
                        div()
                            .whitespace_normal()
                            .type_style(Type::Small)
                            .text_color(rgb(MUTED))
                            .child("Nothing you already have is overwritten. Favorites are pinned."),
                    ),
            )
    }

    fn onepassword_done_body(&self, result: &me_core::OnePasswordSummary) -> Div {
        let breakdown = [
            (result.new, "new"),
            (result.changed, "changed, kept as another version"),
            (result.duplicates, "already in ME, skipped"),
        ]
        .into_iter()
        .filter(|(count, _)| *count > 0)
        .map(|(count, label)| format!("{count} {label}"))
        .collect::<Vec<_>>()
        .join(" · ");
        div()
            .flex()
            .flex_col()
            .gap(px(space::LG))
            .child(
                card()
                    .flex_row()
                    .items_start()
                    .child(icon(Icon::Check, IconSize::Large, SUCCESS))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(space::XS))
                            .child(
                                div()
                                    .type_style(Type::Section)
                                    .child(format!("{} added", entries(result.to_import(), ""))),
                            )
                            .child(
                                div()
                                    .whitespace_normal()
                                    .type_style(Type::Small)
                                    .text_color(rgb(MUTED))
                                    .child(breakdown),
                            ),
                    ),
            )
            .child(
                div()
                    .p(px(space::LG))
                    .rounded(px(radius::STANDARD))
                    .bg(rgb(WARNING_SURFACE))
                    .flex()
                    .items_start()
                    .gap(px(space::SM))
                    .child(icon(Icon::Info, IconSize::Medium, WARNING))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(space::XS))
                            .child(div().type_style(Type::Label).font_weight(font::EMPHASIS).text_color(rgb(WARNING)).child("Delete the export file"))
                            .child(
                                div()
                                    .whitespace_normal()
                                    .type_style(Type::Small)
                                    .text_color(rgb(WARNING))
                                    .child("The .1pux file is not encrypted. After checking your logins, move it to the Trash and empty it."),
                            ),
                    ),
            )
    }

    pub(super) fn open_credential(&mut self, item: u64, cx: &mut Context<Self>) {
        if !self.app_ready() || self.busy {
            return;
        }
        if self.collection.get(item).is_some_and(
            |i| matches!(&i.content, Content::Credential{category,..} if category=="001"),
        ) {
            self.show_login(item, cx);
            return;
        }
        self.clear_credentials(cx);
        self.busy = true;
        self.error = None;
        let generation = self.credential_generation;
        let session = self.session.clone();
        let task = cx.background_executor().spawn(async move {
            session
                .lock()
                .map_err(|_| me_core::Error::Format)?
                .as_ref()
                .ok_or(me_core::Error::Validation("Vault locked."))?
                .credential_details(item)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.credential_generation != generation || !this.app_ready() {
                    return;
                }
                this.busy = false;
                match result {
                    Ok(details) => {
                        if details.native {
                            this.show_login(item, cx);
                        } else {
                            this.credential = Some(details);
                        }
                    }
                    Err(e) => this.notice = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn reveal_credential_field(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.credential_revealed.insert(index) {
            self.credential_revealed.remove(&index);
            cx.notify();
            return;
        }
        let generation = self.credential_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(30))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.credential_generation == generation {
                    this.credential_revealed.remove(&index);
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn pick_credential_export(&mut self, attachment: Option<usize>, cx: &mut Context<Self>) {
        if self.busy || !self.app_ready() {
            return;
        }
        let Some(details) = self.credential.as_ref() else {
            return;
        };
        let item = details.item_id;
        let name = attachment
            .and_then(|i| details.attachments.get(i))
            .map_or("1Password-Original.1pux", |a| a.name.as_str());
        let directory = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        let prompt = cx.prompt_for_new_path(&directory, Some(name));
        let generation = self.credential_generation;
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(path))) = prompt.await {
                let _ = this.update(cx, |this, cx| {
                    if this.credential_generation != generation {
                        return;
                    }
                    this.run_change(cx, move |v| {
                        match attachment {
                            Some(i) => v.export_credential_attachment(item, i, &path)?,
                            None => v.export_credential_original(item, &path)?,
                        }
                        Ok("Exported an unencrypted copy.".into())
                    });
                });
            }
        })
        .detach();
    }

    pub(super) fn credential_modal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(details) = self.credential.as_ref() else {
            return self.overlay(cx);
        };
        self.overlay(cx).child(modal_panel(580., self.motion_enabled()).id("credential-detail").max_h(px(480.)).overflow_y_scroll()
            .child(eyebrow("PRIVATE DETAILS"))
            .child(div().type_style(Type::Title).text_ellipsis().child(details.title.clone()))
            .child(div().type_style(Type::Small).text_color(rgb(MUTED)).child(format!("{} · {}{}",me_core::credential_category(&details.category),details.vault,if details.archived {" · Archived"}else{""})))
            .child(div().w_full().flex_shrink_0().whitespace_normal().type_style(Type::Small).text_color(rgb(MUTED)).child("Revealed and copied values are hidden or cleared after 30 seconds."))
            .child(div().type_style(Type::Caption).text_color(rgb(MUTED)).child(format!("Imported: {}",details.imported_at.replace('T'," ").replace('Z'," UTC"))))
            .children(details.fields.iter().take(200).enumerate().map(|(index,field)|div().p(px(space::MD)).bg(rgb(BG)).rounded(px(radius::STANDARD)).flex().flex_col().gap(px(space::SM))
                .child(div().type_style(Type::Small).font_weight(font::EMPHASIS).child(field.label.clone()))
                .child(div().type_style(Type::Body).font_family(font::MONO).child(if self.credential_revealed.contains(&index) {field.value.to_string()} else {"••••••••".into()}))
                .child(div().flex().gap(px(space::LG)).type_style(Type::Small).text_color(rgb(ACCENT))
                    .child(icon_action(("reveal-credential",index), if self.credential_revealed.contains(&index) { Icon::EyeOff } else { Icon::Eye }, if self.credential_revealed.contains(&index) { "Hide value" } else { "Reveal for 30 seconds" }).on_click(cx.listener(move|this,_,_,cx|this.reveal_credential_field(index,cx))))
                    .child(icon_action(("copy-credential",index), Icon::Copy, "Copy for 30 seconds").on_click(cx.listener(move|this,_,_,cx| {
                        if !this.app_ready() {return;}
                        if let Some(value)=this.credential.as_ref().and_then(|c|c.fields.get(index)) {
                            cx.write_to_clipboard(ClipboardItem::new_string(value.value.to_string()));
                            this.copied_value=Some(zeroize::Zeroizing::new(value.value.to_string()));
                            this.clear_clipboard_later(cx);
                            this.notice=Some("Copied for 30 seconds.".into());cx.notify();
                        }
                    }))))))
            .when(details.fields.len()>200,|s|s.child(div().type_style(Type::Small).child("Showing the first 200 fields. All fields remain in the original export.")))
            .children(details.attachments.iter().enumerate().map(|(index,attachment)|div().id(("export-credential-attachment",index)).type_style(Type::Small).text_color(rgb(ACCENT)).cursor_pointer()
                .on_click(cx.listener(move|this,_,_,cx|this.pick_credential_export(Some(index),cx))).child(format!("Export attachment: {}",attachment.name))))
            .child(div().w_full().flex_shrink_0().whitespace_normal().type_style(Type::Small).text_color(rgb(MUTED)).child("Exports are unencrypted. The original includes every entry and attachment from that import."))
            .child(div().id("export-credential-original").type_style(Type::Small).text_color(rgb(ACCENT)).cursor_pointer().on_click(cx.listener(|this,_,_,cx|this.pick_credential_export(None,cx))).child("Save original export…"))
            .when_some(self.error.clone(),|s,error|s.child(div().type_style(Type::Small).text_color(rgb(DANGER)).child(error)))
            .when_some(self.notice.clone(),|s,n|s.child(div().type_style(Type::Small).text_color(rgb(MUTED)).child(n)))
            .child(div().id("close-credential").type_style(Type::Body).cursor_pointer().on_click(cx.listener(|this,_,window,cx|this.dismiss(&Dismiss,window,cx))).child("Close")))
    }
}

fn entries(count: usize, verb: &str) -> String {
    let noun = if count == 1 { "entry" } else { "entries" };
    if verb.is_empty() {
        format!("{count} {noun}")
    } else {
        format!("{verb} {count} {noun}")
    }
}

fn group_breakdown(group: &me_core::OnePasswordGroup) -> String {
    [
        (group.new, "new"),
        (group.changed, "changed"),
        (group.unchanged, "already in ME"),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, label)| format!("{count} {label}"))
    .collect::<Vec<_>>()
    .join(" · ")
}

fn category_icon(name: &str) -> Icon {
    match name {
        "Login" | "Password" | "Database" | "Server" | "SSH key" | "Wi-Fi" | "API credential" => {
            Icon::Key
        }
        "Credit card" | "Bank account" | "Membership" | "Rewards program" => Icon::Card,
        "Identity" | "Passport" | "Driver's license" | "Social insurance" => Icon::User,
        "Secure note" => Icon::Text,
        "Document" => Icon::Document,
        "Email account" => Icon::Mail,
        _ => Icon::Hash,
    }
}

/// Inset review card inside a dialog.
fn card() -> Div {
    div()
        .p(px(space::LG))
        .rounded(px(radius::STANDARD))
        .bg(rgb(BG))
        .border_1()
        .border_color(rgb(LINE))
        .flex()
        .flex_col()
        .gap(px(space::MD))
}

fn stat(label: &'static str, count: usize, caption: &'static str) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .p(px(space::MD))
        .rounded(px(radius::STANDARD))
        .bg(rgb(SURFACE))
        .border_1()
        .border_color(rgb(LINE))
        .flex()
        .flex_col()
        .gap(px(space::XS))
        .child(
            div()
                .type_style(Type::Caption)
                .text_color(rgb(MUTED))
                .child(label),
        )
        .child(
            div()
                .type_style(Type::Section)
                .text_color(rgb(if count > 0 { INK } else { FAINT }))
                .child(count.to_string()),
        )
        .child(
            div()
                .whitespace_normal()
                .type_style(Type::Caption)
                .text_color(rgb(MUTED))
                .child(caption),
        )
}

const GROUP_ROWS: usize = 20;

fn group_list(
    title: &'static str,
    groups: &[me_core::OnePasswordGroup],
    glyph: Option<Icon>,
) -> Div {
    let hidden = groups.len().saturating_sub(GROUP_ROWS);
    let last = groups.len().min(GROUP_ROWS).saturating_sub(1);
    div()
        .flex()
        .flex_col()
        .gap(px(space::SM))
        .child(eyebrow(title))
        .child(
            div()
                .rounded(px(radius::STANDARD))
                .border_1()
                .border_color(rgb(LINE))
                .flex()
                .flex_col()
                .children(
                    groups
                        .iter()
                        .take(GROUP_ROWS)
                        .enumerate()
                        .map(|(index, group)| {
                            let adding = group.to_import();
                            div()
                                .px(px(space::LG))
                                .py(px(space::MD))
                                .flex()
                                .items_center()
                                .gap(px(space::MD))
                                .when(index < last, |s| s.border_b_1().border_color(rgb(LINE)))
                                .child(icon(
                                    glyph.unwrap_or_else(|| category_icon(&group.name)),
                                    IconSize::Medium,
                                    MUTED,
                                ))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .child(
                                            div()
                                                .type_style(Type::Body)
                                                .text_ellipsis()
                                                .child(group.name.clone()),
                                        )
                                        .child(
                                            div()
                                                .type_style(Type::Caption)
                                                .text_color(rgb(MUTED))
                                                .child(group_breakdown(group)),
                                        ),
                                )
                                .child(if adding > 0 {
                                    div()
                                        .flex_shrink_0()
                                        .type_style(Type::Label)
                                        .font_weight(font::EMPHASIS)
                                        .child(format!("+{adding}"))
                                } else {
                                    div()
                                        .flex_shrink_0()
                                        .type_style(Type::Small)
                                        .text_color(rgb(MUTED))
                                        .child("Nothing new")
                                })
                        }),
                ),
        )
        .when(hidden > 0, |s| {
            s.child(
                div()
                    .type_style(Type::Caption)
                    .text_color(rgb(MUTED))
                    .child(format!("And {hidden} more")),
            )
        })
}
