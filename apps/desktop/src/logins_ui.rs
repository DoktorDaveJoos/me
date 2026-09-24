use super::*;
use crate::design_system::motion as timing;
use gpui::{Div, SharedString, Stateful, uniform_list};
use me_core::{
    LoginDetails, LoginField, LoginPresentation, LoginSummary, LoginUpdate, LoginValueKind,
};
use std::time::Instant;
use zeroize::Zeroizing;

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum LoginScope {
    #[default]
    All,
    Favorites,
    Archived,
}
#[derive(Default)]
pub(super) struct LoginsState {
    pub intake: super::credential_create_ui::CredentialIntake,
    pub creating: Option<me_core::CredentialKind>,
    pub fill_target: Option<Zeroizing<String>>,
    pub fill_after_save: bool,
    pub fill_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub items: Vec<LoginSummary>,
    pub visible: Vec<usize>,
    pub selected: Option<u64>,
    pub details: Option<LoginDetails>,
    pub draft: Option<LoginDraft>,
    pub scope: LoginScope,
    pub loading: bool,
    pub detail_loading: bool,
    pub loaded: bool,
    pub list_request: u64,
    pub detail_request: u64,
    pub revealed: BTreeSet<usize>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub scroll: gpui::UniformListScrollHandle,
    pub detail_scroll: gpui::ScrollHandle,
    pub arrival: Option<Instant>,
    pub icons: super::site_icons::SiteIcons,
    pub reveal_sequence: u64,
    pub reveal_tokens: BTreeMap<usize, u64>,
}
pub(super) struct LoginDraft {
    pub(super) title: Entity<TextInput>,
    pub(super) fields: Vec<(usize, Entity<TextInput>)>,
    favorite: bool,
}
impl MeApp {
    pub(super) fn login_edit_guard(&mut self, cx: &mut Context<Self>) -> bool {
        if self.logins.draft.is_some() || self.logins.intake.open {
            self.logins.notice =
                Some("Save or cancel your changes before leaving this item.".into());
            cx.notify();
            true
        } else {
            false
        }
    }
    pub(super) fn clear_login_draft(&mut self, cx: &mut Context<Self>) {
        self.logins.fill_target = None;
        self.logins.fill_after_save = false;
        if let Some(cancel) = self.logins.fill_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        if let Some(draft) = self.logins.draft.take() {
            draft.title.update(cx, |i, cx| i.set_text("", cx));
            for (_, input) in draft.fields {
                input.update(cx, |i, cx| i.set_text("", cx));
            }
        }
    }
    pub(super) fn clear_logins(&mut self, cx: &mut Context<Self>) {
        self.clear_login_draft(cx);
        self.logins.icons.clear();
        let list_request = self.logins.list_request + 1;
        let detail_request = self.logins.detail_request + 1;
        self.logins = LoginsState {
            list_request,
            detail_request,
            ..Default::default()
        };
        self.login_search.update(cx, |i, cx| i.set_text("", cx));
    }
    pub(super) fn refresh_logins(&mut self, cx: &mut Context<Self>) {
        if !self.app_ready() {
            return;
        }
        self.logins.list_request += 1;
        self.logins.loading = true;
        let request = self.logins.list_request;
        let generation = self.generation;
        let session = self.session.clone();
        let task = cx.background_executor().spawn(async move {
            session
                .lock()
                .map_err(|_| me_core::Error::Format)?
                .as_ref()
                .ok_or(me_core::Error::Format)?
                .logins()
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation || this.logins.list_request != request {
                    return;
                }
                this.logins.loading = false;
                this.logins.loaded = true;
                match result {
                    Ok(items) => {
                        this.logins.items = items;
                        this.filter_logins(cx);
                    }
                    Err(_) => this.logins.error = Some("Couldn't load logins. Try again.".into()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn filter_logins(&mut self, cx: &mut Context<Self>) {
        let query = self.login_search.read(cx).content.to_lowercase();
        self.logins.visible = self
            .logins
            .items
            .iter()
            .enumerate()
            .filter(|(_, i)| {
                (match self.logins.scope {
                    LoginScope::All => true,
                    LoginScope::Favorites => i.favorite,
                    LoginScope::Archived => i.archived,
                }) && query.split_whitespace().all(|q| {
                    i.title.to_lowercase().contains(q) || i.vault.to_lowercase().contains(q)
                })
            })
            .map(|(i, _)| i)
            .collect();
        if !self.logins.loading
            && self.logins.draft.is_none()
            && !self.logins.intake.open
            && !self
                .logins
                .visible
                .iter()
                .any(|&i| Some(self.logins.items[i].id) == self.logins.selected)
        {
            if let Some(&i) = self.logins.visible.first() {
                self.select_login(self.logins.items[i].id, cx);
            } else {
                self.logins.selected = None;
                self.logins.details = None;
                self.logins.revealed.clear();
                self.logins.detail_request += 1;
                self.logins.detail_loading = false;
            }
        }
        cx.notify();
    }
    pub(super) fn show_login(&mut self, item: u64, cx: &mut Context<Self>) {
        if self.login_edit_guard(cx) || !self.app_ready() || self.busy {
            return;
        }
        self.clear_credentials(cx);
        self.page = Page::Logins;
        self.show_settings = false;
        self.show_add = false;
        self.show_onepassword = false;
        self.logins.scope = LoginScope::All;
        self.login_search.update(cx, |i, cx| i.set_text("", cx));
        self.select_login(item, cx);
        self.refresh_logins(cx);
    }
    pub(super) fn select_login(&mut self, item: u64, cx: &mut Context<Self>) {
        if !self.app_ready() || self.busy || self.login_edit_guard(cx) {
            return;
        }
        if self.logins.selected != Some(item) {
            self.logins.arrival = Some(Instant::now());
        }
        self.logins.detail_request += 1;
        self.logins
            .detail_scroll
            .set_offset(gpui::point(px(0.), px(0.)));
        self.logins.selected = Some(item);
        self.logins.details = None;
        self.logins.revealed.clear();
        self.logins.error = None;
        self.logins.notice = None;
        self.logins.detail_loading = true;
        let request = self.logins.detail_request;
        let generation = self.generation;
        let session = self.session.clone();
        let task = cx.background_executor().spawn(async move {
            session
                .lock()
                .map_err(|_| me_core::Error::Format)?
                .as_ref()
                .ok_or(me_core::Error::Format)?
                .login_details(item)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation || this.logins.detail_request != request {
                    return;
                }
                this.logins.detail_loading = false;
                match result {
                    Ok(details) => this.logins.details = Some(details),
                    Err(_) => {
                        this.logins.error = Some("Couldn't open this login. Try again.".into())
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn edit_login(
        &mut self,
        _: &EditLogin,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.page != Page::Logins
            || self.show_settings
            || self.show_onepassword
            || self.logins.intake.open
            || self.busy
            || self.logins.draft.is_some()
        {
            return;
        }
        let Some(details) = &self.logins.details else {
            return;
        };
        let title = cx.new(|cx| {
            let mut i = TextInput::new("Item title", cx);
            i.set_text(&details.credential.title, cx);
            i
        });
        let fields = details
            .fields
            .iter()
            .enumerate()
            .filter(|(_, f)| f.editable)
            .map(|(n, f)| {
                let input = cx.new(|cx| {
                    let mut i = if f.multiline {
                        TextInput::multiline(&f.label, cx)
                    } else {
                        TextInput::new(&f.label, cx)
                    };
                    i.set_concealed(f.concealed, cx);
                    i.set_text(&f.value, cx);
                    i
                });
                (n, input)
            })
            .collect();
        self.logins
            .detail_scroll
            .set_offset(gpui::point(px(0.), px(0.)));
        window.focus(&title.focus_handle(cx));
        self.logins.draft = Some(LoginDraft {
            title,
            fields,
            favorite: details.favorite,
        });
        self.logins.revealed.clear();
        self.logins.notice = None;
        self.logins.error = None;
        cx.notify();
    }
    pub(super) fn cancel_login_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.clear_login_draft(cx);
        if self.logins.creating.take().is_some() {
            self.logins.details = None;
        }
        self.logins.notice = None;
        self.logins.error = None;
        self.logins.revealed.clear();
        window.focus(&self.login_focus);
        // Reload so a revision conflict can be resolved without leaving the page.
        if let Some(id) = self.logins.selected {
            self.select_login(id, cx);
        }
        cx.notify();
    }
    pub(super) fn save_login(
        &mut self,
        _: &SaveLogin,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.page != Page::Logins || !self.app_ready() || self.busy {
            return;
        }
        let (Some(details), Some(draft)) = (&self.logins.details, &self.logins.draft) else {
            return;
        };
        let fill_target = if std::mem::take(&mut self.logins.fill_after_save) {
            self.logins.fill_target.clone()
        } else {
            None
        };
        if let Some(target) = &fill_target {
            let website = draft
                .fields
                .iter()
                .find(|(i, _)| details.fields[*i].key == "website")
                .map(|(_, input)| input.read(cx).content.to_string())
                .unwrap_or_default();
            if !super::context_source::fill_matches_website(target, &website) {
                self.logins.error = Some(
                    "The website was changed. Use Save, or select the page again before filling."
                        .into(),
                );
                cx.notify();
                return;
            }
            let target_data: serde_json::Value = serde_json::from_str(target).unwrap_or_default();
            let mut required = vec!["username", "password"];
            if target_data["name_required"] == true {
                required.push("full_name");
            }
            for key in required {
                if !draft.fields.iter().any(|(i, input)| {
                    details.fields[*i].key == key && !input.read(cx).content.is_empty()
                }) {
                    self.logins.error = Some(
                        if key == "full_name" {
                            "This page requires your name. Add it to the draft before filling."
                        } else {
                            "Add an email and password before saving and filling."
                        }
                        .into(),
                    );
                    cx.notify();
                    return;
                }
            }
        }
        let item = details.credential.item_id;
        let creating = self.logins.creating;
        let update = LoginUpdate {
            revision: details.revision,
            title: draft.title.read(cx).content.to_string(),
            favorite: draft.favorite,
            fields: draft
                .fields
                .iter()
                .filter_map(|(i, input)| {
                    let f = &details.fields[*i];
                    let text = &input.read(cx).content;
                    (text.as_ref() != f.value.as_str())
                        .then(|| (f.key.clone(), Zeroizing::new(text.to_string())))
                })
                .collect(),
        };
        let concealed = details
            .fields
            .iter()
            .enumerate()
            .filter(|(_, f)| f.concealed)
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        for index in concealed {
            self.mask_login_editor(index, true, cx);
        }
        self.busy = true;
        self.logins.error = None;
        self.logins.notice = None;
        self.logins.revealed.clear();
        window.focus(&self.login_focus);
        let session = self.session.clone();
        let generation = self.generation;
        let request = self.logins.detail_request;
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.logins.fill_cancel = Some(cancel.clone());
        let task = cx.background_executor().spawn(async move {
            let mut session = session.lock().map_err(|_| me_core::Error::Format)?;
            let vault = session.as_mut().ok_or(me_core::Error::Format)?;
            let item = if let Some(kind) = creating {
                vault.create_credential(kind, update)?
            } else {
                vault.update_login(item, update)?;
                item
            };
            let details = vault.login_details(item)?;
            drop(session);
            let filled = fill_target.map(|target| {
                let fields = details
                    .fields
                    .iter()
                    .filter(|f| ["username", "password", "full_name"].contains(&f.key.as_str()))
                    .map(|f| (f.key.clone(), f.value.clone()))
                    .collect::<Vec<_>>();
                super::context_source::fill(&target, &fields, &cancel)
            });
            Ok::<_, me_core::Error>((details, filled))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation || this.logins.detail_request != request {
                    return;
                }
                this.busy = false;
                match result {
                    Ok((details, filled)) => {
                        this.clear_login_draft(cx);
                        this.logins.creating = None;
                        this.logins.selected = Some(details.credential.item_id);
                        this.logins.scope = LoginScope::All;
                        this.login_search.update(cx, |i, cx| i.set_text("", cx));
                        this.logins.details = Some(details);
                        this.logins.notice = Some(
                            match filled {
                                Some(Ok(())) => {
                                    "Login saved and form filled. Review and submit on the website."
                                }
                                Some(Err(message)) => message,
                                None => "Item saved.",
                            }
                            .into(),
                        );
                        this.refresh_logins(cx);
                        this.refresh_search(cx);
                    }
                    Err(me_core::Error::Validation(message)) => {
                        this.logins.error = Some(message.into())
                    }
                    Err(_) => {
                        this.logins.error = Some(
                            "Couldn't save this item. Your draft is still here; try again.".into(),
                        )
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn login_tab(
        &mut self,
        backwards: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(draft) = &self.logins.draft else {
            cx.propagate();
            return;
        };
        let handles: Vec<_> = std::iter::once(&draft.title)
            .chain(draft.fields.iter().map(|(_, i)| i))
            .map(|i| i.focus_handle(cx))
            .collect();
        let index = handles
            .iter()
            .position(|h| h.is_focused(window))
            .unwrap_or(0);
        let next = if backwards {
            (index + handles.len() - 1) % handles.len()
        } else {
            (index + 1) % handles.len()
        };
        window.focus(&handles[next]);
        let inputs: Vec<_> = std::iter::once(&draft.title)
            .chain(draft.fields.iter().map(|(_, i)| i))
            .collect();
        if let Some(bounds) = inputs[next].read(cx).bounds() {
            let viewport = self.logins.detail_scroll.bounds();
            let offset = self.logins.detail_scroll.offset();
            let top = viewport.top() + px(space::XXL);
            let bottom = viewport.bottom() - px(space::XXL);
            if bounds.top() < top {
                self.logins
                    .detail_scroll
                    .set_offset(gpui::point(offset.x, offset.y + top - bounds.top()));
            } else if bounds.bottom() > bottom {
                self.logins
                    .detail_scroll
                    .set_offset(gpui::point(offset.x, offset.y + bottom - bounds.bottom()));
            }
        }
        cx.notify();
    }
    pub(super) fn move_login(
        &mut self,
        direction: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.logins.draft.is_some() || self.logins.visible.is_empty() {
            return;
        }
        let index = self
            .logins
            .visible
            .iter()
            .position(|&i| Some(self.logins.items[i].id) == self.logins.selected)
            .unwrap_or(0);
        let next = index
            .saturating_add_signed(direction)
            .min(self.logins.visible.len() - 1);
        self.logins
            .scroll
            .scroll_to_item(next, gpui::ScrollStrategy::Top);
        let id = self.logins.items[self.logins.visible[next]].id;
        if self.logins.selected != Some(id) {
            self.select_login(id, cx);
        }
        window.focus(&self.login_focus);
    }
    fn mask_login_editor(&mut self, index: usize, concealed: bool, cx: &mut Context<Self>) {
        if let Some(input) = self
            .logins
            .draft
            .as_ref()
            .and_then(|d| d.fields.iter().find(|(i, _)| *i == index))
            .map(|(_, i)| i)
        {
            input.update(cx, |i, cx| i.set_concealed(concealed, cx));
        }
    }
    fn reveal_login(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.busy || !self.app_ready() {
            return;
        }
        if !self.logins.revealed.insert(index) {
            self.logins.revealed.remove(&index);
            self.logins.reveal_tokens.remove(&index);
            self.mask_login_editor(index, true, cx);
            cx.notify();
            return;
        }
        self.mask_login_editor(index, false, cx);
        self.logins.reveal_sequence += 1;
        let token = self.logins.reveal_sequence;
        self.logins.reveal_tokens.insert(index, token);
        let request = self.logins.detail_request;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(30))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.logins.detail_request == request
                    && this.logins.reveal_tokens.get(&index) == Some(&token)
                {
                    this.logins.revealed.remove(&index);
                    this.mask_login_editor(index, true, cx);
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }
    fn copy_login(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.app_ready() {
            return;
        }
        if let Some(field) = self
            .logins
            .details
            .as_ref()
            .and_then(|d| d.fields.get(index))
        {
            cx.write_to_clipboard(ClipboardItem::new_string(field.value.to_string()));
            self.copied_value = Some(field.value.clone());
            self.clear_clipboard_later(cx);
            self.logins.notice = Some("Copied for 30 seconds.".into());
            cx.notify();
        }
    }
    fn export_login_file(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        if self.busy || !self.app_ready() || self.logins.draft.is_some() {
            return;
        }
        let Some(details) = &self.logins.details else {
            return;
        };
        let name = index
            .and_then(|i| details.credential.attachments.get(i))
            .map_or("1Password-original.1pux", |a| a.name.as_str());
        let item = details.credential.item_id;
        let directory = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        let prompt = cx.prompt_for_new_path(&directory, Some(name));
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(path))) = prompt.await {
                let _ = this.update(cx, |this, cx| {
                    if this.generation == generation {
                        this.run_change(cx, move |v| {
                            match index {
                                Some(index) => {
                                    v.export_credential_attachment(item, index, &path)?
                                }
                                None => v.export_credential_original(item, &path)?,
                            }
                            Ok("Exported an unencrypted copy.".into())
                        });
                    }
                });
            }
        })
        .detach();
    }
    fn login_site_icon(
        &mut self,
        website: &str,
        cx: &mut Context<Self>,
    ) -> Option<Arc<gpui::RenderImage>> {
        if !self.motion.loaded || !self.motion.website_icons || !self.app_ready() {
            return None;
        }
        let host = super::site_icons::site_host(website)?;
        if let Some(cached) = self.logins.icons.lookup(&host) {
            return cached;
        }
        if self.logins.icons.pending.len() >= super::site_icons::MAX_PENDING
            || !self.logins.icons.pending.insert(host.clone())
        {
            return None;
        }
        let cancel = self.logins.icons.cancel.clone();
        let worker_cancel = cancel.clone();
        let worker_host = host.clone();
        let task = cx
            .background_executor()
            .spawn(async move { super::site_icons::fetch(&worker_host, &worker_cancel) });
        cx.spawn(async move |this, cx| {
            let image = task.await;
            let _ = this.update(cx, |this, cx| {
                if !Arc::ptr_eq(&cancel, &this.logins.icons.cancel)
                    || !this.motion.website_icons
                    || !this.app_ready()
                {
                    return;
                }
                this.logins.icons.pending.remove(&host);
                this.logins.icons.insert(host, image);
                cx.notify();
            });
        })
        .detach();
        None
    }
    fn login_row(&mut self, index: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let item = self.logins.items[self.logins.visible[index]].clone();
        let logo = self.login_site_icon(&item.website, cx);
        let id = item.id;
        let active = self.logins.selected == Some(id);
        let phase = motion::phase(
            self.logins.arrival,
            timing::LOGIN_ENTER_MS,
            self.motion_enabled(),
        );
        div()
            .id(SharedString::from(format!("login-{id}")))
            .relative()
            .w_full()
            .min_w_0()
            .h(px(layout::LOGIN_ROW_HEIGHT))
            .px(px(space::MD))
            .py(px(space::SM))
            .rounded(px(radius::STANDARD))
            .bg(rgb(if active { SURFACE } else { BG }))
            .border_1()
            .border_color(rgb(if active { LINE } else { BG }))
            .hover(|s| s.bg(rgb(HOVER)))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                // Clicking the current row keeps its details and motion settled.
                if this.logins.selected != Some(id) {
                    this.select_login(id, cx);
                }
                window.focus(&this.login_focus);
            }))
            .flex()
            .items_center()
            .gap(px(space::MD))
            .when(active && phase < 1., |s| {
                s.child(motion::frame_at(motion::Frame::Navigation, phase))
            })
            .child(motion::login_badge(active, phase, logo))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .type_style(Type::Body)
                            .font_weight(if active { font::EMPHASIS } else { font::MEDIUM })
                            .text_ellipsis()
                            .child(item.title.clone()),
                    )
                    .child(
                        div()
                            .type_style(Type::Caption)
                            .text_color(rgb(MUTED))
                            .text_ellipsis()
                            .child(format!(
                                "{}{}{}",
                                item.vault,
                                if item.archived { " · Archived" } else { "" },
                                if item.versions > 1 {
                                    format!(" · Version {}/{}", item.version, item.versions)
                                } else {
                                    String::new()
                                }
                            )),
                    ),
            )
            .when(item.favorite, |s| {
                s.child(icon(Icon::Star, IconSize::Small, MUTED))
            })
    }
    pub(super) fn logins_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let website = self
            .logins
            .items
            .iter()
            .find(|item| {
                self.logins.creating.is_none()
                    && !self.logins.intake.open
                    && Some(item.id) == self.logins.selected
            })
            .map(|item| item.website.clone())
            .unwrap_or_default();
        let logo = self.login_site_icon(&website, cx);
        let phase = motion::phase(
            self.logins.arrival,
            timing::LOGIN_ENTER_MS,
            self.motion_enabled(),
        );
        if phase < 1. {
            window.request_animation_frame();
        }
        let compact = window.viewport_size().width < px(layout::LOGIN_COMPACT_BREAKPOINT);
        div()
            .size_full()
            .min_h_0()
            .flex()
            .child(
                div()
                    .w(px(if compact {
                        layout::LOGIN_LIST_COMPACT_WIDTH
                    } else {
                        layout::LOGIN_LIST_WIDTH
                    }))
                    .h_full()
                    .flex_shrink_0()
                    .bg(rgb(BG))
                    .border_r_1()
                    .border_color(rgb(LINE))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .px(px(space::LG))
                            .pt(px(space::PAGE_TOP))
                            .pb(px(space::LG))
                            .flex()
                            .flex_col()
                            .gap(px(space::LG))
                            .child(heading("Logins"))
                            .child(
                                div()
                                    .flex()
                                    .gap(px(space::SM))
                                    .child(
                                        primary_action()
                                            .id("new-credential")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.new_credential(&NewCredential, window, cx)
                                            }))
                                            .child(icon(Icon::Plus, IconSize::Medium, SURFACE))
                                            .child("New item"),
                                    )
                                    .child(
                                        secondary_action()
                                            .id("login-import")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                if !this.login_edit_guard(cx) {
                                                    this.pick_onepassword(cx);
                                                }
                                            }))
                                            .child("Import"),
                                    ),
                            )
                            .child(
                                div()
                                    .id("login-search-field")
                                    .h(px(layout::CONTROL_LARGE))
                                    .px(px(space::MD))
                                    .rounded(px(radius::STANDARD))
                                    .border_1()
                                    .border_color(rgb(LINE))
                                    .bg(rgb(SURFACE))
                                    .flex()
                                    .items_center()
                                    .gap(px(space::SM))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        window.focus(&this.login_search.focus_handle(cx))
                                    }))
                                    .child(icon(Icon::Search, IconSize::Medium, MUTED))
                                    .child(if self.logins.draft.is_some() {
                                        self.login_search.read(cx).frozen().into_any_element()
                                    } else {
                                        self.login_search.clone().into_any_element()
                                    }),
                            )
                            .child(
                                div().flex().gap(px(space::XS)).children(
                                    [
                                        (LoginScope::All, "All"),
                                        (LoginScope::Favorites, "Favorites"),
                                        (LoginScope::Archived, "Archive"),
                                    ]
                                    .into_iter()
                                    .map(|(scope, label)| {
                                        div()
                                            .id(SharedString::from(format!("login-scope-{label}")))
                                            .h(px(layout::CONTROL_COMPACT))
                                            .px(px(space::SM))
                                            .rounded(px(radius::STANDARD))
                                            .flex()
                                            .items_center()
                                            .type_style(Type::Small)
                                            .bg(rgb(if self.logins.scope == scope {
                                                SURFACE
                                            } else {
                                                BG
                                            }))
                                            .text_color(rgb(if self.logins.scope == scope {
                                                INK
                                            } else {
                                                MUTED
                                            }))
                                            .cursor_pointer()
                                            .hover(|s| s.bg(rgb(HOVER)))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if !this.login_edit_guard(cx) {
                                                    this.logins.scope = scope;
                                                    this.filter_logins(cx);
                                                }
                                            }))
                                            .child(label)
                                    }),
                                ),
                            )
                            .child(eyebrow(if self.logins.loading {
                                "Loading…".into()
                            } else {
                                format!(
                                    "{} {}",
                                    self.logins.visible.len(),
                                    if self.logins.visible.len() == 1 {
                                        "item"
                                    } else {
                                        "items"
                                    }
                                )
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .px(px(space::SM))
                            .track_focus(&self.login_focus)
                            .key_context("LoginsList")
                            .on_action(cx.listener(|this, _: &NextLogin, window, cx| {
                                this.move_login(1, window, cx)
                            }))
                            .on_action(cx.listener(|this, _: &PreviousLogin, window, cx| {
                                this.move_login(-1, window, cx)
                            }))
                            .child(
                                uniform_list(
                                    "logins-list",
                                    self.logins.visible.len(),
                                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                                        range.map(|i| this.login_row(i, cx)).collect::<Vec<_>>()
                                    }),
                                )
                                .w_full()
                                .track_scroll(self.logins.scroll.clone())
                                .h_full(),
                            ),
                    )
                    .when(
                        self.logins.visible.is_empty() && !self.logins.loading,
                        |s| {
                            s.child(
                                div()
                                    .p(px(space::LG))
                                    .type_style(Type::Small)
                                    .text_color(rgb(MUTED))
                                    .child(if self.logins.items.is_empty() {
                                        "Create an item or import your 1Password export."
                                    } else {
                                        "No matching logins."
                                    }),
                            )
                        },
                    ),
            )
            .child(self.login_detail_view(phase, logo, cx))
    }
    fn login_detail_view(
        &self,
        phase: f32,
        logo: Option<Arc<gpui::RenderImage>>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let mut panel = div()
            .id("login-detail-scroll")
            .track_scroll(&self.logins.detail_scroll)
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .px(px(space::XXL))
            .pt(px(space::PAGE_TOP))
            .pb(px(space::PAGE));
        if let Some(details) = &self.logins.details {
            let draft = self.logins.draft.as_ref();
            let favorite = draft.map_or(details.favorite, |d| d.favorite);
            let content = div()
                .w_full()
                .max_w(px(layout::CONTENT_WIDTH))
                .mx_auto()
                .flex()
                .flex_col()
                .gap(px(space::XXL))
                .child(
                    div()
                        .relative()
                        .p(px(space::LG))
                        .rounded(px(radius::STANDARD))
                        .border_1()
                        .border_color(rgb(LINE))
                        .bg(rgb(SURFACE))
                        .flex()
                        .flex_col()
                        .gap(px(space::MD))
                        .child(motion::login_identity(phase, logo))
                        .child(eyebrow(format!(
                            "{} · {}{}",
                            details.credential.vault,
                            me_core::credential_category(&details.credential.category),
                            if details.credential.archived {
                                " · Archived"
                            } else {
                                ""
                            }
                        )))
                        .child(if let Some(draft) = draft {
                            self.login_input_frame(&draft.title, false, cx)
                                .into_any_element()
                        } else {
                            heading(details.credential.title.clone())
                                .whitespace_normal()
                                .into_any_element()
                        })
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap(px(space::SM))
                                .when(draft.is_none(), |s| {
                                    s.child(
                                        secondary_action()
                                            .id("edit-login")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.edit_login(&EditLogin, window, cx)
                                            }))
                                            .child("Edit"),
                                    )
                                })
                                .when(draft.is_some(), |s| {
                                    s.child(
                                        primary_action()
                                            .id("save-login")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.save_login(&SaveLogin, window, cx)
                                            }))
                                            .child(if self.busy { "Saving…" } else { "Save" }),
                                    )
                                    .when(self.logins.fill_target.is_some(), |s| s.child(
                                        primary_action().id("save-and-fill-login")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                if !this.busy { this.logins.fill_after_save = true; this.save_login(&SaveLogin, window, cx); }
                                            }))
                                            .child(if self.busy { "Saving and filling…" } else { "Save and fill" })
                                    ))
                                    .child(
                                        secondary_action()
                                            .id("cancel-login-edit")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.cancel_login_edit(window, cx)
                                            }))
                                            .child("Cancel"),
                                    )
                                })
                                .child(
                                    div()
                                        .id("login-favorite")
                                        .h(px(layout::CONTROL))
                                        .px(px(space::SM))
                                        .flex()
                                        .items_center()
                                        .gap(px(space::SM))
                                        .type_style(Type::Small)
                                        .text_color(rgb(if favorite { ACCENT } else { MUTED }))
                                        .when(draft.is_some(), |s| {
                                            s.cursor_pointer().on_click(cx.listener(
                                                |this, _, _, cx| {
                                                    if !this.busy {
                                                        if let Some(draft) = &mut this.logins.draft
                                                        {
                                                            draft.favorite = !draft.favorite;
                                                        }
                                                        cx.notify();
                                                    }
                                                },
                                            ))
                                        })
                                        .child(icon(
                                            Icon::Star,
                                            IconSize::Medium,
                                            if favorite { ACCENT } else { MUTED },
                                        ))
                                        .child(if favorite {
                                            "Favorite"
                                        } else if draft.is_some() {
                                            "Add to favorites"
                                        } else {
                                            "Not favorited"
                                        }),
                                ),
                        ),
                )
                .when_some(self.logins.error.clone(), |s, e| {
                    s.child(
                        div()
                            .p(px(space::MD))
                            .rounded(px(radius::STANDARD))
                            .bg(rgb(DANGER_SURFACE))
                            .type_style(Type::Small)
                            .text_color(rgb(DANGER))
                            .child(e),
                    )
                })
                .when_some(self.logins.notice.clone(), |s, n| {
                    s.child(
                        div()
                            .type_style(Type::Small)
                            .text_color(rgb(MUTED))
                            .child(n),
                    )
                })
                .children(details.fields.iter().enumerate()
                    .filter(|(_, field)| draft.is_some() || !field.value.is_empty())
                    .map(|(index, field)| {
                        let input = draft.and_then(|d| d.fields.iter().find(|(i, _)| *i == index)).map(|(_, input)| input);
                        self.login_field_view(field, index, input,
                            index == 0 || details.fields[index - 1].section != field.section,
                            phase, cx)
                    }))
                .when(!details.credential.attachments.is_empty(), |s| {
                    s.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(space::MD))
                            .child(eyebrow("Attachments"))
                            .children(details.credential.attachments.iter().enumerate().map(
                                |(index, a)| {
                                    secondary_action()
                                        .id(("login-attachment", index))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.export_login_file(Some(index), cx)
                                        }))
                                        .child(icon(Icon::Attach, IconSize::Medium, MUTED))
                                        .child(
                                            div().min_w_0().text_ellipsis().child(a.name.clone()),
                                        )
                                },
                            ))
                            .child(
                                div()
                                    .type_style(Type::Caption)
                                    .text_color(rgb(MUTED))
                                    .child("Saving an attachment creates an unencrypted copy."),
                            ),
                    )
                })
                .when(draft.is_none() && details.native_kind.is_none(), |s| s.child(div().flex().flex_col().gap(px(space::SM))
                    .child(secondary_action().id("export-login-original").on_click(cx.listener(|this,_,_,cx|this.export_login_file(None,cx))).child("Save original export…"))
                    .child(div().type_style(Type::Caption).text_color(rgb(MUTED)).child("Unencrypted. Includes every entry from this import, before your ME. edits."))))
                .child(
                    div()
                        .type_style(Type::Caption)
                        .text_color(rgb(MUTED))
                        .whitespace_normal()
                        .child(format!(
                            "{} {} · Revision {}\nSecrets hide again after 30 seconds.",
                            if self.logins.creating.is_some() { "Unsaved draft" } else if details.native_kind.is_some() { "Created" } else { "Imported" },
                            details
                                .credential
                                .imported_at
                                .replace('T', " ")
                                .replace('Z', " UTC"),
                            details.revision
                        )),
                );
            panel = panel.child(content);
        } else {
            panel = panel.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::LG))
                    .child(motion::login_identity(phase, None))
                    .child(heading(if self.logins.detail_loading {
                        "Opening login…"
                    } else if self.logins.items.is_empty() {
                        "Your logins, together."
                    } else if self.logins.visible.is_empty() {
                        "No matching logins"
                    } else {
                        "Select a login"
                    }))
                    .child(div().type_style(Type::Body).text_color(rgb(MUTED)).child(
                        if self.logins.detail_loading {
                            "Reading this login from your vault."
                        } else if self.logins.items.is_empty() {
                            "Import your 1Password export to browse and edit your logins here."
                        } else if self.logins.visible.is_empty() {
                            "Try a different name, vault, or filter."
                        } else {
                            "Choose a login from the list to see its details."
                        },
                    ))
                    .when_some(self.logins.error.clone(), |s, e| {
                        s.child(
                            div()
                                .type_style(Type::Small)
                                .text_color(rgb(DANGER))
                                .child(e),
                        )
                        .child(
                            secondary_action()
                                .id("retry-logins")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.logins.error = None;
                                    if let Some(id) = this.logins.selected {
                                        this.select_login(id, cx);
                                    } else {
                                        this.refresh_logins(cx);
                                    }
                                }))
                                .child("Try again"),
                        )
                    }),
            );
        }
        panel
    }
    fn login_field_view(
        &self,
        field: &LoginField,
        index: usize,
        input: Option<&Entity<TextInput>>,
        section_start: bool,
        phase: f32,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let visible = !field.concealed || self.logins.revealed.contains(&index);
        let compact = input.is_none()
            && matches!(
                field.presentation,
                LoginPresentation::Tags | LoginPresentation::Label | LoginPresentation::Number
            );
        let copyable = !matches!(
            field.presentation,
            LoginPresentation::Tags | LoginPresentation::Label | LoginPresentation::Number
        );
        let mut body = div().flex().flex_col().gap(px(space::SM));
        if !compact {
            body = body
                .p(px(space::LG))
                .rounded(px(radius::STANDARD))
                .bg(rgb(SURFACE))
                .border_1()
                .border_color(rgb(LINE));
        }
        body =
            body.child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .gap(px(space::SM))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .type_style(Type::Small)
                            .text_color(rgb(MUTED))
                            .child(field.label.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_shrink_0()
                            .gap(px(space::MD))
                            .when(
                                input.is_some()
                                    && (field.key == "password" || field.label == "Password"),
                                |s| {
                                    s.child(
                                        div()
                                            .id(("generate-password", index))
                                            .type_style(Type::Small)
                                            .text_color(rgb(ACCENT))
                                            .cursor_pointer()
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.generate_login_password(index, cx)
                                            }))
                                            .child("Generate"),
                                    )
                                },
                            )
                            .when(field.concealed, |s| {
                                s.child(
                                    div()
                                        .id(("login-reveal", index))
                                        .type_style(Type::Small)
                                        .text_color(rgb(ACCENT))
                                        .cursor_pointer()
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.reveal_login(index, cx)
                                        }))
                                        .child(if visible { "Hide" } else { "Reveal" }),
                                )
                            })
                            .when(input.is_none() && copyable, |s| {
                                s.child(
                                    div()
                                        .id(("login-copy", index))
                                        .type_style(Type::Small)
                                        .text_color(rgb(ACCENT))
                                        .cursor_pointer()
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.copy_login(index, cx)
                                        }))
                                        .child("Copy"),
                                )
                            }),
                    ),
            );
        if field.presentation == LoginPresentation::History {
            body = body.child(
                div()
                    .type_style(Type::Caption)
                    .text_color(rgb(MUTED))
                    .whitespace_normal()
                    .child(history_caption(field)),
            );
        }
        body = body.child(if let Some(input) = input {
            self.login_input_frame(input, field.multiline, cx)
                .into_any_element()
        } else if matches!(
            field.presentation,
            LoginPresentation::Tags | LoginPresentation::Label
        ) {
            let labels: Vec<_> = if field.presentation == LoginPresentation::Tags {
                field.value.lines().collect()
            } else {
                vec![field.value.as_str()]
            };
            div()
                .flex()
                .flex_wrap()
                .gap(px(space::SM))
                .children(labels.into_iter().map(|label| tag_chip(label.to_owned())))
                .into_any_element()
        } else {
            div()
                .type_style(if compact { Type::Value } else { Type::Body })
                .font_family(font::MONO)
                .whitespace_normal()
                .overflow_hidden()
                .child(if visible {
                    field.value.to_string()
                } else {
                    "••••••••".into()
                })
                .into_any_element()
        });
        div()
            .opacity(motion::login_content_opacity(phase, index))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap(px(space::SM))
            .when(
                section_start && field.presentation != LoginPresentation::Tags,
                |s| s.child(eyebrow(field.section.clone())),
            )
            .child(
                body.when(
                    input.is_some()
                        && field.kind == LoginValueKind::Json
                        && field.presentation != LoginPresentation::Number,
                    |s| {
                        s.child(
                            div()
                                .type_style(Type::Caption)
                                .text_color(rgb(MUTED))
                                .child("Structured value · JSON"),
                        )
                    },
                )
                .when(input.is_some() && field.kind == LoginValueKind::Tags, |s| {
                    s.child(
                        div()
                            .type_style(Type::Caption)
                            .text_color(rgb(MUTED))
                            .child("One tag per line"),
                    )
                }),
            )
            .into_any_element()
    }
    pub(super) fn login_input_frame(
        &self,
        input: &Entity<TextInput>,
        multiline: bool,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let focus = input.focus_handle(cx);
        div()
            .track_focus(&focus)
            .focus(|s| s.border_color(rgb(FOCUS)))
            .id(SharedString::from(format!(
                "login-input-{}",
                input.entity_id()
            )))
            .on_click(move |_, window, _| window.focus(&focus))
            .w_full()
            .min_w_0()
            .px(px(space::MD))
            .py(px(space::SM))
            .min_h(px(layout::CONTROL_LARGE))
            .when(multiline, |s| {
                s.max_h(px(layout::LOGIN_TEXTAREA_HEIGHT))
                    .overflow_y_scroll()
            })
            .rounded(px(radius::STANDARD))
            .border_1()
            .border_color(rgb(LINE))
            .bg(rgb(SURFACE))
            .flex()
            .items_center()
            .child(if self.busy {
                input.read(cx).frozen().into_any_element()
            } else {
                input.clone().into_any_element()
            })
    }
    pub(super) fn login_import_modal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        self.overlay(cx).child(
            modal_panel(688., self.motion_enabled())
                .max_h(px(520.))
                .id("login-import-modal")
                .overflow_y_scroll()
                .child(self.onepassword_settings(cx))
                .child(
                    secondary_action()
                        .id("close-login-import")
                        .on_click(cx.listener(|this, _, _, cx| {
                            if !this.busy {
                                this.show_onepassword = false;
                                this.clear_credentials(cx);
                                this.refresh_logins(cx);
                                cx.notify();
                            }
                        }))
                        .child("Done"),
                ),
        )
    }
}

fn history_caption(field: &LoginField) -> String {
    match (field.used_until, field.used_until_date.as_deref()) {
        (Some(time), Some(date)) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            format!("Used until {date} · {}", history_age(time, now))
        }
        _ => "Date no longer current is unavailable in this export.".into(),
    }
}
fn history_age(time: i64, now: i64) -> String {
    if time > now {
        return "date is in the future".into();
    }
    let elapsed = now.saturating_sub(time);
    for (unit, seconds) in [
        ("year", 31_536_000),
        ("month", 2_592_000),
        ("day", 86_400),
        ("hour", 3_600),
        ("minute", 60),
    ] {
        let count = elapsed / seconds;
        if count > 0 {
            return format!("{count} {unit}{} ago", if count == 1 { "" } else { "s" });
        }
    }
    "just now".into()
}
#[cfg(test)]
mod detail_tests {
    use super::*;
    #[test]
    fn history_age_is_honest_for_recent_old_and_future_timestamps() {
        assert_eq!(history_age(1000, 1000), "just now");
        assert_eq!(history_age(1000, 1060), "1 minute ago");
        assert_eq!(history_age(1000, 173800), "2 days ago");
        assert_eq!(history_age(1000, 31537000), "1 year ago");
        assert_eq!(history_age(1001, 1000), "date is in the future");
    }
}
