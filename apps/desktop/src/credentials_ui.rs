use super::*;
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

impl MeApp {
    pub(super) fn clear_credentials(&mut self, cx: &mut Context<Self>) {
        self.credential_generation += 1;
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
        self.show_onepassword = true;
        self.clear_credentials(cx);
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
        self.busy = true;
        self.error = None;
        self.notice = Some("Checking the 1Password export…".into());
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
                this.notice = None;
                match result {
                    Ok(preview) => this.onepassword_preview = Some(preview),
                    Err(e) => this.error = Some(e.to_string()),
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
                        if this.page == Page::Logins {
                            this.refresh_logins(cx);
                        }
                        this.refresh_search(cx);
                    }
                    Err(error) => {
                        this.error = Some(error.to_string());
                        this.onepassword_preview = Some((import, summary));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn onepassword_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div().w(px(640.)).max_w_full().min_w_0().flex_shrink_0().p(px(space::XXL)).bg(rgb(SURFACE)).border_1().border_color(rgb(LINE)).rounded(px(radius::STANDARD)).flex().flex_col().gap(px(space::MD))
            .child(eyebrow("1PASSWORD"))
            .child(div().type_style(Type::Section).font_weight(font::EMPHASIS).child("Import from 1Password"))
            .child(div().max_w(px(592.)).flex_shrink_0().whitespace_normal().type_style(Type::Body).text_color(rgb(MUTED)).child("In 1Password 8, choose File → Export → 1PUX, then select that file here."))
            .child(div().max_w(px(592.)).flex_shrink_0().whitespace_normal().type_style(Type::Small).text_color(rgb(MUTED)).child("Logins, notes, cards, custom fields, password history, and attachments are stored locally and excluded from AI."))
            .child(div().max_w(px(592.)).flex_shrink_0().whitespace_normal().type_style(Type::Small).text_color(rgb(MUTED)).child("Passkeys aren't included. Use 1PUX; CSV and 1PIF aren't supported. The export is unencrypted—delete it after checking the import."))
            .child(primary_action().id("pick-onepassword").flex_shrink_0()
                .hover(|s| s.bg(rgb(PRIMARY_HOVER))).on_click(cx.listener(|this,_,_,cx|this.pick_onepassword(cx))).child(if self.busy {"Please wait…"} else {"Choose export…"}))
            .when_some(self.onepassword_result.as_ref(),|s,result|s.child(div().p(px(space::LG)).rounded(px(radius::STANDARD)).bg(rgb(BG)).flex_shrink_0().flex().flex_col().gap(px(space::SM))
                .child(div().type_style(Type::Label).font_weight(font::EMPHASIS).child("Import complete"))
                .child(div().max_w(px(560.)).whitespace_normal().type_style(Type::Body).child(format!("{} imported · {} unchanged · {} changed versions kept",result.to_import(),result.duplicates,result.changed)))
                .child(div().max_w(px(560.)).whitespace_normal().type_style(Type::Small).text_color(rgb(MUTED)).child("Check your imported entries, then delete the unencrypted export."))
                .child(div().id("show-imported-credentials").type_style(Type::Body).text_color(rgb(ACCENT)).cursor_pointer().on_click(cx.listener(|this,_,window,cx|this.navigate(Page::Logins,window,cx))).child("View logins"))))
            .when_some(self.error.clone(),|s,error|s.child(div().max_w(px(560.)).whitespace_normal().type_style(Type::Small).text_color(rgb(DANGER)).child(error)))
            .when_some(self.onepassword_preview.as_ref(),|s,(_,summary)|s
                .child(div().p(px(space::LG)).rounded(px(radius::STANDARD)).bg(rgb(BG)).flex().flex_col().gap(px(space::SM))
                    .child(div().type_style(Type::Label).font_weight(font::EMPHASIS).child("Review import"))
                    .child(div().type_style(Type::Body).child(format!("{} entries · {} new · {} unchanged · {} changed",summary.total,summary.new,summary.duplicates,summary.changed)))
                    .child(div().type_style(Type::Small).text_color(rgb(MUTED)).child(format!("{} archived entries · {} files in the original export",summary.archived,summary.files)))
                    .children(summary.vaults.iter().take(20).map(|(name,count)|div().type_style(Type::Small).text_ellipsis().child(format!("{name} · {count} entries"))))
                    .child(div().max_w(px(592.)).flex_shrink_0().whitespace_normal().type_style(Type::Small).text_color(rgb(MUTED)).child("Duplicates are skipped. Changed entries are saved as additional versions."))
                    .when(summary.to_import()>0,|s|s.child(primary_action().id("confirm-onepassword").flex_shrink_0()
                        .hover(|s| s.bg(rgb(PRIMARY_HOVER))).on_click(cx.listener(|this,_,_,cx|this.commit_onepassword(cx))).child(format!("Import {} entries",summary.to_import()))))
                    .when(summary.to_import()==0,|s|s.child(div().type_style(Type::Body).child(if summary.total==0 {"This export is empty."} else {"All entries are already in ME."})))
                    .child(div().id("cancel-onepassword").type_style(Type::Small).cursor_pointer().on_click(cx.listener(|this,_,_,cx|{this.clear_credentials(cx);cx.notify();})).child("Cancel"))))
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
