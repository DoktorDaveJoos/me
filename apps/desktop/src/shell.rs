use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[path = "workspace_ui.rs"]
mod workspace_ui;
use workspace_ui::Page;
#[path = "knowledge_canvas.rs"]
mod knowledge_canvas;
#[path = "knowledge_ui.rs"]
mod knowledge_ui;
use knowledge_ui::KnowledgeState;
#[path = "interface_preferences.rs"]
mod interface_preferences;
#[path = "motion_ui.rs"]
mod motion_ui;
use motion_ui::MotionPreferences;
#[path = "ai_ui.rs"]
mod ai_ui;
#[path = "import_ui.rs"]
mod import_ui;
use import_ui::{ActiveImport, PendingFile};
#[path = "chat_ui.rs"]
mod file_import_ui;
#[path = "filter_ui.rs"]
mod filter_ui;
use filter_ui::FilterState;
#[path = "account_client.rs"]
mod account_client;
#[path = "account_ui.rs"]
mod account_ui;
#[path = "codex_setup_ui.rs"]
mod codex_setup_ui;
#[path = "context_source.rs"]
mod context_source;
#[path = "credential_capture.rs"]
mod credential_capture;
#[path = "credential_create_ui.rs"]
mod credential_create_ui;
#[path = "credential_tools_ui.rs"]
mod credential_tools_ui;
#[path = "credentials_ui.rs"]
mod credentials_ui;
#[path = "logins_ui.rs"]
mod logins_ui;
#[path = "site_icons.rs"]
mod site_icons;
use logins_ui::LoginsState;
#[cfg(any(debug_assertions, feature = "development-tools"))]
#[path = "development_ui.rs"]
mod development_ui;
#[path = "device_account_ui.rs"]
mod device_account_ui;
#[path = "device_accounts.rs"]
mod device_accounts;
#[path = "onboarding_ui.rs"]
mod onboarding_ui;
#[path = "permissions_ui.rs"]
mod permissions_ui;
#[path = "question_ui.rs"]
mod question_ui;
#[path = "settings_ui.rs"]
mod settings_ui;
use account_ui::{AccountMode, AccountState};
#[path = "vault_ui.rs"]
mod vault_ui;

use crate::{assets::icon, input::TextInput, theme::*};
use gpui::{
    App, ClipboardItem, Context, Entity, ExternalPaths, FocusHandle, Focusable, PathPromptOptions,
    Window, actions, div, prelude::*, px, rgb, rgba,
};
use me_core::{Collection, Content, Vault};

actions!(
    me,
    [
        LockVault,
        OpenSettings,
        OpenPermissions,
        FocusSearch,
        AddFact,
        Confirm,
        Dismiss,
        NextField,
        PreviousField,
        NewCredential,
        EditLogin,
        SaveLogin,
        NextLogin,
        PreviousLogin
    ]
);

pub struct MeApp {
    focus: FocusHandle,
    search: Entity<TextInput>,
    filter_input: Entity<TextInput>,
    page: Page,
    knowledge: KnowledgeState,
    motion: MotionPreferences,
    knowledge_input: Entity<TextInput>,
    knowledge_focus: FocusHandle,
    focus_filter_on_ready: bool,
    filter: FilterState,
    attachments: Vec<u64>,
    reviews: Vec<me_core::ReviewEntry>,
    selected_reviews: BTreeSet<String>,
    folders: BTreeMap<u64, String>,
    expanded_folders: BTreeSet<String>,
    organize_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    organization_attempted: bool,
    organization_error: Option<String>,
    title_input: Entity<TextInput>,
    value_input: Entity<TextInput>,
    collection: Collection,
    editing: Option<Option<u64>>,
    show_add: bool,
    show_info: bool,
    show_settings: bool,
    permissions: permissions_ui::PermissionsState,
    #[cfg(any(debug_assertions, feature = "development-tools"))]
    development: development_ui::DevelopmentState,
    show_onepassword: bool,
    onepassword_preview: Option<(me_core::OnePasswordImport, me_core::OnePasswordSummary)>,
    onepassword_result: Option<me_core::OnePasswordSummary>,
    credential: Option<me_core::CredentialDetails>,
    credential_revealed: std::collections::BTreeSet<usize>,
    credential_generation: u64,
    logins: LoginsState,
    login_search: Entity<TextInput>,
    login_focus: FocusHandle,
    settings: me_core::AppSettings,
    settings_saving: bool,
    auto_queue_check: bool,
    ai_item: Option<u64>,
    active_imports: BTreeMap<u64, ActiveImport>,
    import_jobs: Vec<me_core::ImportJob>,
    expanded_imports: BTreeSet<u64>,
    import_pause: Option<String>,
    import_refresh: bool,
    import_refresh_pending: bool,
    pending_imports: Vec<PendingFile>,
    pending_attach_to_search: bool,
    import_scans: usize,
    intake_active: bool,
    intake_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    intake_files: Vec<PathBuf>,
    intake_message: Option<String>,
    intake_failures: Vec<(PathBuf, String)>,
    pending_generation: u64,
    document_error: Option<String>,
    document_warning: Option<String>,
    ai_warning: Option<(u64, String)>,
    pinned_only: bool,
    notice: Option<String>,
    error: Option<String>,
    copied: Option<u64>,
    session: Arc<Mutex<Option<Vault>>>,
    root: Option<PathBuf>,
    password: Entity<TextInput>,
    password_repeat: Entity<TextInput>,
    account_email: Entity<TextInput>,
    recovery_input: Entity<TextInput>,
    account: AccountState,
    initialized: bool,
    unlocked: bool,
    busy: bool,
    generation: u64,
    search_generation: u64,
    copied_value: Option<zeroize::Zeroizing<String>>,
    clipboard_generation: u64,
    document_open: Option<u64>,
    restore_from: Option<PathBuf>,
    history: Vec<me_core::Revision>,
    bridge: Option<me_agent::BridgeServer>,
    ai_message: Option<String>,
    ai_failure: Option<(Option<u64>, String)>,
    proposals: Vec<me_core::Proposal>,
    questions: Vec<me_core::ReviewQuestion>,
    question_edit: Option<(String, Entity<TextInput>)>,
    question_error: Option<(String, String)>,
    proposals_generation: u64,
    codex_ready: bool,
    codex_issue: Option<me_agent::codex::SetupIssue>,
    codex_notice: bool,
    show_codex_setup: bool,
    focus_password_on_ready: bool,
    codex_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    codex_generation: u64,
    codex_message: String,
    codex_login_url: Option<String>,
    codex_login_requested: bool,
}

impl MeApp {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextInput::new("Find a file…", cx));
        let filter_input = cx.new(TextInput::filter);
        let login_search = cx.new(|cx| TextInput::new("Search logins…", cx));
        cx.observe(&login_search, |this, _, cx| this.filter_logins(cx))
            .detach();
        let knowledge_input = cx.new(|cx| TextInput::new("Find anything in your knowledge…", cx));
        cx.observe(&knowledge_input, |this, _, cx| {
            this.knowledge_query_changed(cx)
        })
        .detach();
        let title_input =
            cx.new(|cx| TextInput::new("e.g. Passport number, shoe size, or an idea", cx));
        let value_input = cx.new(|cx| TextInput::new("A value or a short note", cx));
        let password = cx.new(TextInput::password);
        let password_repeat = cx.new(|cx| TextInput::secret("Repeat master password", cx));
        let account_email = cx.new(|cx| TextInput::new("you@example.com", cx));
        let recovery_input = cx.new(|cx| TextInput::secret("Your saved recovery code", cx));
        cx.observe(&search, |_, _, cx| cx.notify()).detach();
        cx.observe(&filter_input, |this, _, cx| this.filter_changed(cx))
            .detach();
        Self::load_motion_preferences(cx);
        Self::inspect_vault(cx);
        Self {
            focus: cx.focus_handle(),
            search,
            filter_input,
            page: Page::Search,
            knowledge: KnowledgeState::default(),
            motion: MotionPreferences::default(),
            knowledge_input,
            knowledge_focus: cx.focus_handle(),
            focus_filter_on_ready: true,
            filter: FilterState::default(),
            attachments: Vec::new(),
            reviews: Vec::new(),
            selected_reviews: BTreeSet::new(),
            folders: BTreeMap::new(),
            expanded_folders: BTreeSet::new(),
            organize_cancel: None,
            organization_attempted: false,
            organization_error: None,
            title_input,
            value_input,
            collection: Collection::default(),
            editing: None,
            show_add: false,
            show_info: false,
            show_settings: false,
            permissions: Default::default(),
            #[cfg(any(debug_assertions, feature = "development-tools"))]
            development: Default::default(),
            show_onepassword: false,
            onepassword_preview: None,
            onepassword_result: None,
            credential: None,
            credential_revealed: Default::default(),
            credential_generation: 0,
            logins: LoginsState::default(),
            login_search,
            login_focus: cx.focus_handle(),
            settings: me_core::AppSettings::default(),
            settings_saving: false,
            auto_queue_check: false,
            ai_item: None,
            active_imports: BTreeMap::new(),
            import_jobs: Vec::new(),
            expanded_imports: BTreeSet::new(),
            import_pause: None,
            import_refresh: false,
            import_refresh_pending: false,
            pending_imports: Vec::new(),
            pending_attach_to_search: false,
            import_scans: 0,
            intake_active: false,
            intake_cancel: None,
            intake_files: Vec::new(),
            intake_message: None,
            intake_failures: Vec::new(),
            pending_generation: 0,
            document_error: None,
            document_warning: None,
            ai_warning: None,
            pinned_only: false,
            notice: None,
            error: None,
            copied: None,
            session: Arc::new(Mutex::new(None)),
            root: Self::vault_path(),
            password,
            password_repeat,
            account_email,
            recovery_input,
            account: AccountState::default(),
            initialized: false,
            unlocked: false,
            busy: true,
            generation: 0,
            search_generation: 0,
            copied_value: None,
            clipboard_generation: 0,
            document_open: None,
            restore_from: None,
            history: Vec::new(),
            bridge: None,
            ai_message: None,
            ai_failure: None,
            proposals: Vec::new(),
            questions: Vec::new(),
            question_edit: None,
            question_error: None,
            proposals_generation: 0,
            codex_ready: false,
            codex_issue: None,
            codex_notice: false,
            show_codex_setup: false,
            focus_password_on_ready: true,
            codex_cancel: None,
            codex_generation: 0,
            codex_message: "Checking connection…".into(),
            codex_login_url: None,
            codex_login_requested: false,
        }
    }

    fn focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        if self.login_edit_guard(cx) {
            return;
        }
        if self.app_ready()
            && !self.busy
            && self.credential.is_none()
            && self.editing.is_none()
            && !self.show_info
            && !self.show_settings
            && !self.show_codex_setup
        {
            self.show_add = false;
            window.focus(&if self.page == Page::Knowledge {
                self.knowledge_input.focus_handle(cx)
            } else if self.page == Page::Logins {
                self.login_search.focus_handle(cx)
            } else if self.page == Page::Browser {
                self.search.focus_handle(cx)
            } else {
                self.filter_input.focus_handle(cx)
            });
            cx.notify();
        }
    }

    fn add_fact(&mut self, _: &AddFact, window: &mut Window, cx: &mut Context<Self>) {
        if self.login_edit_guard(cx) {
            return;
        }
        if self.app_ready()
            && !self.busy
            && self.credential.is_none()
            && self.editing.is_none()
            && !self.show_info
            && !self.show_settings
            && !self.show_codex_setup
        {
            self.open_editor(None, window, cx);
        }
    }

    fn open_editor(&mut self, id: Option<u64>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.app_ready() || self.busy {
            return;
        }
        self.history.clear();
        if let Some(id) = id {
            self.load_history(id, cx);
        }
        let (title, value) = id
            .and_then(|id| self.collection.get(id))
            .and_then(|item| {
                if let Content::Note(value) = &item.content {
                    Some((item.title.clone(), value.clone()))
                } else {
                    None
                }
            })
            .unwrap_or_default();
        self.title_input
            .update(cx, |input, cx| input.set_text(&title, cx));
        self.value_input
            .update(cx, |input, cx| input.set_text(&value, cx));
        self.editing = Some(id);
        self.show_add = false;
        self.error = None;
        window.focus(&if id.is_some() {
            self.value_input.focus_handle(cx)
        } else {
            self.title_input.focus_handle(cx)
        });
        cx.notify();
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if self.permissions.open {
            return;
        }
        if self.provider_setup_visible() {
            self.finish_onboarding(cx);
            return;
        }
        if !self.unlocked {
            window.focus(&self.focus);
            self.unlock_vault(cx);
            return;
        }
        if self.show_codex_setup {
            return;
        }
        if !self.pending_imports.is_empty() {
            self.confirm_imports(cx);
            return;
        }
        if self.busy {
            return;
        }
        if let (Some(item), Some((question, input))) =
            (self.document_open, self.question_edit.as_ref())
        {
            let question = question.clone();
            let value = input.read(cx).content.to_string();
            window.focus(&self.focus);
            self.answer_question(item, question, Some(value), cx);
            return;
        }
        let Some(id) = self.editing else {
            if self.page == Page::Search
                && !self.show_info
                && !self.show_settings
                && !self.show_add
                && self.document_open.is_none()
            {
                self.submit_filter(window, cx);
            }
            return;
        };
        let title = self.title_input.read(cx).content.to_string();
        let value = zeroize::Zeroizing::new(self.value_input.read(cx).content.to_string());
        self.run_change(cx, move |vault| {
            vault.save_note(id, &title, &value)?;
            Ok("Saved.".into())
        });
        window.focus(&self.focus);
    }

    fn next_field(&mut self, _: &NextField, window: &mut Window, cx: &mut Context<Self>) {
        if self.permissions.open {
            return;
        }
        if !self.app_ready() && self.busy {
            return;
        }
        if !self.unlocked && self.account_setup_visible() {
            self.account_focus(window, cx, false);
        } else if !self.unlocked {
            let next = if !self.initialized
                && self.restore_from.is_none()
                && self.password.focus_handle(cx).is_focused(window)
            {
                self.password_repeat.focus_handle(cx)
            } else {
                self.password.focus_handle(cx)
            };
            window.focus(&next);
        } else if self.logins.draft.is_some() {
            self.login_tab(false, window, cx);
        } else if self.editing.is_some() {
            let next = if self.title_input.focus_handle(cx).is_focused(window) {
                self.value_input.focus_handle(cx)
            } else {
                self.title_input.focus_handle(cx)
            };
            window.focus(&next);
        } else {
            cx.propagate();
        }
    }

    fn previous_field(&mut self, _: &PreviousField, window: &mut Window, cx: &mut Context<Self>) {
        if self.permissions.open {
            return;
        }
        if !self.app_ready() && self.busy {
            return;
        }
        if !self.unlocked && self.account_setup_visible() {
            self.account_focus(window, cx, true);
        } else if self.logins.draft.is_some() {
            self.login_tab(true, window, cx);
        } else {
            self.next_field(&NextField, window, cx);
        }
    }

    fn dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        if self.permissions.open {
            self.close_permissions(cx);
            return;
        }
        #[cfg(any(debug_assertions, feature = "development-tools"))]
        if self.show_settings && self.development.confirming {
            self.development.confirming = false;
            cx.notify();
            return;
        }
        if self.show_codex_setup {
            self.show_codex_setup = false;
            window.focus(&self.filter_input.focus_handle(cx));
            cx.notify();
            return;
        }
        if !self.pending_imports.is_empty() || self.import_scans > 0 {
            self.cancel_import_confirmation(cx);
            return;
        }
        if self.busy {
            return;
        }
        if self.password_generator_open().is_some() {
            self.close_password_generator(window, cx);
            return;
        }
        if self.page == Page::Logins && !self.show_settings {
            if self.logins.intake.open {
                self.close_credential_intake(cx);
                return;
            }
            if self.logins.draft.is_some() {
                self.cancel_login_edit(window, cx);
                return;
            }
            if self.show_onepassword {
                self.show_onepassword = false;
                self.clear_credentials(cx);
                cx.notify();
                return;
            }
        }
        if self.filter.handoff_review {
            self.filter.handoff_review = false;
            cx.notify();
            return;
        }
        let had_overlay = self.editing.is_some()
            || self.show_add
            || self.show_info
            || self.show_settings
            || self.document_open.is_some()
            || self.credential.is_some();
        self.clear_credentials(cx);
        self.document_open = None;
        self.questions.clear();
        self.question_edit = None;
        self.question_error = None;
        self.history.clear();
        self.title_input
            .update(cx, |input, cx| input.set_text("", cx));
        self.value_input
            .update(cx, |input, cx| input.set_text("", cx));
        self.editing = None;
        self.show_add = false;
        self.show_info = false;
        self.show_settings = false;
        self.error = None;
        self.notice = None;
        if !had_overlay {
            self.page = Page::Search;
            self.search.update(cx, |input, cx| input.set_text("", cx));
        }
        window.focus(&self.filter_input.focus_handle(cx));
        cx.notify();
    }

    fn pick_documents(&mut self, cx: &mut Context<Self>) {
        if !self.app_ready() || self.busy {
            return;
        }
        self.show_add = false;
        cx.notify();
        let generation = self.generation;
        let attach_to_search = self.page == Page::Search && !self.show_settings;
        let task = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Add".into()),
        });
        cx.spawn(async move |this, cx| match task.await {
            Ok(Ok(Some(paths))) => {
                let _ = this.update(cx, |this, cx| {
                    if this.generation == generation {
                        this.pending_attach_to_search |= attach_to_search;
                        this.accept_documents(&paths, cx);
                    }
                });
            }
            Ok(Ok(None)) => {}
            _ => {
                let _ = this.update(cx, |this, cx| {
                    this.notice = Some("Couldn't open the file picker.".into());
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn overlay(&self, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .absolute()
            .inset_0()
            .occlude()
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.accept_documents(paths.paths(), cx)
            }))
            .drag_over::<ExternalPaths>(|s, _, _, _| s.border_2().border_color(rgb(ACCENT)))
            .bg(rgba(SCRIM))
            .flex()
            .items_center()
            .justify_center()
    }

    fn edit_modal(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.overlay(cx).child(
            modal_panel(486., self.motion_enabled())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(eyebrow("ABOUT YOU"))
                        .child(
                            div()
                                .id("close-editor")
                                .size(px(24.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.dismiss(&Dismiss, window, cx)
                                }))
                                .child(icon(Icon::Close, IconSize::Medium, MUTED)),
                        ),
                )
                .child(heading(if self.editing.flatten().is_some() {
                    "Edit detail"
                } else {
                    "Add a detail"
                }))
                .child(self.editor_field("Label", self.title_input.clone(), window, cx))
                .child(
                    div()
                        .type_style(Type::Caption)
                        .text_color(rgb(MUTED))
                        .child("Give it a name you'll remember."),
                )
                .child(self.editor_field("Value or note", self.value_input.clone(), window, cx))
                .when(!self.history.is_empty(), |s| {
                    s.child(
                        div()
                            .type_style(Type::Caption)
                            .text_color(rgb(MUTED))
                            .text_ellipsis()
                            .child(format!(
                                "History: {}",
                                self.history
                                    .iter()
                                    .take(3)
                                    .map(|r| format!(
                                        "{} ({})",
                                        r.value,
                                        if r.state == "accept" {
                                            "confirmed"
                                        } else {
                                            "replaced"
                                        }
                                    ))
                                    .collect::<Vec<_>>()
                                    .join(" · ")
                            )),
                    )
                })
                .when_some(self.error.clone(), |s, error| {
                    s.child(
                        div()
                            .type_style(Type::Small)
                            .text_color(rgb(DANGER))
                            .child(error),
                    )
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .type_style(Type::Caption)
                                .text_color(rgb(MUTED))
                                .child("Saved locally"),
                        )
                        .child(
                            primary_action()
                                .id("save-note")
                                .hover(|s| s.bg(rgb(PRIMARY_HOVER)))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.confirm(&Confirm, window, cx)
                                }))
                                .child("Save")
                                .child(icon(Icon::Arrow, IconSize::Medium, SURFACE)),
                        ),
                ),
        )
    }

    fn editor_field(
        &self,
        label: &'static str,
        input: Entity<TextInput>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let frozen = !self.app_ready() && self.busy;
        let focused = !frozen && input.focus_handle(cx).is_focused(window);
        let focus_input = input.clone();
        div()
            .flex()
            .flex_col()
            .gap(px(space::SM))
            .child(
                div()
                    .type_style(Type::Small)
                    .text_color(rgb(MUTED))
                    .child(label),
            )
            .child(
                div()
                    .id(label)
                    .relative()
                    .when(focused, |s| {
                        s.child(motion::frame(motion::Frame::Focus, self.motion_enabled()))
                    })
                    .w_full()
                    .h(px(layout::CONTROL_LARGE))
                    .px(px(space::MD))
                    .rounded(px(radius::STANDARD))
                    .border_1()
                    .border_color(rgb(if focused { FOCUS } else { LINE }))
                    .flex()
                    .items_center()
                    .on_click(move |_, window, cx| {
                        if !frozen {
                            window.focus(&focus_input.focus_handle(cx));
                        }
                    })
                    .child(if frozen {
                        input.read(cx).frozen().into_any_element()
                    } else {
                        input.into_any_element()
                    }),
            )
    }

    fn add_modal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        self.overlay(cx).child(
            modal_panel(410., self.motion_enabled())
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .items_center()
                        .child(heading("Add to ME."))
                        .child(
                            div()
                                .id("close-add")
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.dismiss(&Dismiss, window, cx)
                                }))
                                .child(icon(Icon::Close, IconSize::Medium, MUTED)),
                        ),
                )
                .child(
                    div()
                        .id("add-onepassword")
                        .type_style(Type::Body)
                        .text_color(rgb(ACCENT))
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_add = false;
                            this.show_settings = true;
                            this.pick_onepassword(cx);
                        }))
                        .child("Import from 1Password…"),
                )
                .child(
                    div()
                        .id("add-file")
                        .p(px(space::LG))
                        .border_1()
                        .border_color(rgb(LINE))
                        .rounded(px(radius::STANDARD))
                        .flex()
                        .items_center()
                        .gap(px(space::MD))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(HOVER)))
                        .on_click(cx.listener(|this, _, _, cx| this.pick_documents(cx)))
                        .child(icon(Icon::Upload, IconSize::Large, ACCENT))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(space::SM))
                                .child(div().type_style(Type::Label).child("Add files"))
                                .child(
                                    div()
                                        .type_style(Type::Caption)
                                        .text_color(rgb(MUTED))
                                        .child("Documents, photos, receipts…"),
                                ),
                        ),
                )
                .child(
                    div()
                        .id("add-note")
                        .p(px(space::LG))
                        .border_1()
                        .border_color(rgb(LINE))
                        .rounded(px(radius::STANDARD))
                        .flex()
                        .items_center()
                        .gap(px(space::MD))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(HOVER)))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_editor(None, window, cx)),
                        )
                        .child(icon(Icon::Text, IconSize::Large, ACCENT))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(space::SM))
                                .child(div().type_style(Type::Label).child("Add a detail"))
                                .child(
                                    div()
                                        .type_style(Type::Caption)
                                        .text_color(rgb(MUTED))
                                        .child("A number, a detail, a note…"),
                                ),
                        ),
                ),
        )
    }

    fn info_modal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        self.overlay(cx).child(modal_panel(430., self.motion_enabled())
            .child(div().type_style(Type::Body).text_color(rgb(MUTED)).child("Share your current, released sources with Codex until you lock ME. Shared content may remain in Codex history."))
            .child(div().id("toggle-codex-access").type_style(Type::Body).text_color(rgb(ACCENT)).cursor_pointer().on_click(cx.listener(|this,_,_,cx|this.toggle_codex(cx))).child(if self.bridge.is_some(){"Stop access"}else{"Allow access this session"}))
            .child(div().type_style(Type::Caption).text_color(rgb(MUTED)).child("Connect the bundled me-mcp server in Codex. Credentials and unreleased files are excluded."))
            .when_some(self.notice.clone(),|s,n|s.child(div().type_style(Type::Small).text_color(rgb(MUTED)).child(n)))
            .child(div().id("backup-vault").type_style(Type::Body).text_color(rgb(ACCENT)).cursor_pointer().on_click(cx.listener(|this,_,_,cx|this.pick_backup(cx))).child("Back up vault…"))
            .child(primary_action().id("close-info")
                .hover(|s| s.bg(rgb(PRIMARY_HOVER))).on_click(cx.listener(|this, _, window, cx| this.dismiss(&Dismiss, window, cx))).child("Done")))
    }
}

impl Focusable for MeApp {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for MeApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(any(debug_assertions, feature = "development-tools"))]
        if self.development.wiping {
            return self.wipe_progress().into_any_element();
        }
        if !self.app_ready() {
            if self.busy {
                window.focus(&self.focus);
            }
            if !self.unlocked && !self.busy && self.focus_password_on_ready {
                if self.account_setup_visible()
                    && self.account.pending.is_none()
                    && self.account.complete.is_none()
                {
                    window.focus(&self.account_email.focus_handle(cx));
                } else if !self.account_setup_visible() {
                    window.focus(&self.password.focus_handle(cx));
                }
                self.focus_password_on_ready = false;
            }
            let content = if self.provider_setup_visible() {
                window.focus(&self.focus);
                self.account_view(window, cx).into_any_element()
            } else {
                self.locked_view(window, cx).into_any_element()
            };
            return div()
                .id("import-locked-shell")
                .bg(rgb(BG))
                .font_family(font::SANS)
                .type_style(Type::Body)
                .text_color(rgb(INK))
                .size_full()
                .relative()
                .when(self.motion.loaded, |s| {
                    s.child(motion::field(
                        motion::Field::Identity,
                        self.motion_enabled(),
                    ))
                })
                .when(self.busy && self.motion.loaded, |s| {
                    s.child(motion::activity(
                        motion::Field::Identity,
                        self.motion_enabled(),
                    ))
                })
                .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                    this.accept_documents(paths.paths(), cx)
                }))
                .drag_over::<ExternalPaths>(|s, _, _, _| s.border_2().border_color(rgb(ACCENT)))
                .child(content)
                .on_action(cx.listener(Self::open_permissions))
                .when(self.permissions.open, |s| {
                    s.child(self.permissions_modal(window, cx))
                })
                .when(self.unlocked, |s| {
                    s.on_action(cx.listener(Self::lock_vault))
                })
                .when(cfg!(debug_assertions), |s| {
                    s.child(
                        div()
                            .absolute()
                            .right(px(space::LG))
                            .bottom(px(space::LG))
                            .child(
                                secondary_action()
                                    .id("locked-motion-toggle")
                                    .hover(|s| s.bg(rgb(HOVER)))
                                    .on_click(cx.listener(|this, _, _, cx| this.toggle_motion(cx)))
                                    .child(if self.motion.saving {
                                        "Saving…"
                                    } else if self.motion.reduced {
                                        "Enable animations"
                                    } else {
                                        "Reduce motion"
                                    }),
                            ),
                    )
                })
                .when(
                    !self.pending_imports.is_empty() || self.import_scans > 0,
                    |s| {
                        s.child(
                            div()
                                .absolute()
                                .bottom(px(space::XXL))
                                .left(px(space::XXL))
                                .right(px(space::XXL))
                                .p(px(space::MD))
                                .bg(rgb(SURFACE))
                                .rounded(px(radius::STANDARD))
                                .type_style(Type::Body)
                                .font_family(font::SANS)
                                .text_color(rgb(INK))
                                .child("Files waiting. Unlock ME. to review your import."),
                        )
                    },
                )
                .into_any_element();
        }
        if self.show_codex_setup || !self.pending_imports.is_empty() || self.import_scans > 0 {
            window.focus(&self.focus);
        } else if self.focus_filter_on_ready {
            window.focus(&self.filter_input.focus_handle(cx));
            self.focus_filter_on_ready = false;
        }
        div()
            .id("me-shell")
            .relative()
            .size_full()
            .flex()
            .bg(rgb(BG))
            .font_family(font::SANS)
            .type_style(Type::Body)
            .text_color(rgb(INK))
            .track_focus(&self.focus)
            .key_context("Me")
            .on_action(cx.listener(Self::lock_vault))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::open_permissions))
            .on_action(cx.listener(Self::focus_search))
            .on_action(cx.listener(Self::add_fact))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::dismiss))
            .on_action(cx.listener(Self::next_field))
            .on_action(cx.listener(Self::previous_field))
            .on_action(cx.listener(Self::new_credential))
            .on_action(cx.listener(Self::edit_login))
            .on_action(cx.listener(Self::save_login))
            .on_action(cx.listener(Self::paste_files))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.accept_documents(paths.paths(), cx)
            }))
            .drag_over::<ExternalPaths>(|s, _, _, _| s.border_2().border_color(rgb(ACCENT)))
            .child(self.sidebar(cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    // GPUI paints siblings in order: decoration stays below every page.
                    .child(
                        div()
                            .id((
                                "workspace-decoration",
                                if self.show_settings {
                                    6
                                } else {
                                    self.page as usize
                                },
                            ))
                            .absolute()
                            .inset_0()
                            .child(motion::field(
                                motion::Field::Workspace,
                                self.motion_enabled(),
                            ))
                            .when(self.workspace_activity(), |s| {
                                s.child(motion::activity(
                                    motion::Field::Workspace,
                                    self.motion_enabled(),
                                ))
                            }),
                    )
                    .when(self.codex_notice && self.codex_issue.is_some(), |s| {
                        s.child(self.codex_notice_view(cx))
                    })
                    .child(motion::page(
                        if self.show_settings {
                            self.settings_view(cx).into_any_element()
                        } else {
                            match self.page {
                                Page::Search => self.filter_view(window, cx).into_any_element(),
                                Page::Review => self.review_view(cx).into_any_element(),
                                Page::Browser => self.browser_view(cx).into_any_element(),
                                Page::Knowledge => {
                                    self.knowledge_view(window, cx).into_any_element()
                                }
                                Page::Imports => self.import_view(cx).into_any_element(),
                                Page::Logins => self.logins_view(window, cx).into_any_element(),
                            }
                        },
                        if self.show_settings {
                            6
                        } else {
                            self.page as usize
                        },
                        self.motion_enabled(),
                    )),
            )
            .when(self.filter.handoff_review, |s| {
                s.child(motion::overlay(
                    self.handoff_modal(cx).into_any_element(),
                    "overlay-handoff",
                    self.motion_enabled(),
                ))
            })
            .when(
                self.page == Page::Logins && self.show_onepassword && !self.show_settings,
                |s| s.child(self.login_import_modal(cx)),
            )
            .when(self.logins.intake.open && !self.show_settings, |s| {
                s.child(motion::overlay(
                    self.credential_intake_modal(window, cx),
                    "overlay-new-credential",
                    self.motion_enabled(),
                ))
            })
            .when(
                self.password_generator_open().is_some() && !self.show_settings,
                |s| {
                    s.child(motion::overlay(
                        self.password_generator_modal(window, cx).into_any_element(),
                        "overlay-password-generator",
                        self.motion_enabled(),
                    ))
                },
            )
            .when(self.credential.is_some(), |s| {
                s.child(motion::overlay(
                    self.credential_modal(cx).into_any_element(),
                    "overlay-credential",
                    self.motion_enabled(),
                ))
            })
            .when(self.document_open.is_some(), |s| {
                s.child(motion::overlay(
                    self.document_modal(cx).into_any_element(),
                    "overlay-document",
                    self.motion_enabled(),
                ))
            })
            .when(self.show_add, |s| {
                s.child(motion::overlay(
                    self.add_modal(cx).into_any_element(),
                    "overlay-add",
                    self.motion_enabled(),
                ))
            })
            .when(self.show_info, |s| {
                s.child(motion::overlay(
                    self.info_modal(cx).into_any_element(),
                    "overlay-info",
                    self.motion_enabled(),
                ))
            })
            .when(self.editing.is_some(), |s| {
                s.child(motion::overlay(
                    self.edit_modal(window, cx).into_any_element(),
                    "overlay-edit",
                    self.motion_enabled(),
                ))
            })
            .when(
                !self.pending_imports.is_empty() || self.import_scans > 0,
                |s| {
                    s.child(motion::overlay(
                        self.import_confirmation(cx).into_any_element(),
                        "overlay-import-confirmation",
                        self.motion_enabled(),
                    ))
                },
            )
            .when(self.show_codex_setup, |s| {
                s.child(motion::overlay(
                    self.codex_setup_view(cx).into_any_element(),
                    "overlay-codex-setup",
                    self.motion_enabled(),
                ))
            })
            .when(self.permissions.open, |s| {
                s.child(self.permissions_modal(window, cx))
            })
            .into_any_element()
    }
}
