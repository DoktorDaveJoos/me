//! Menu bar quick panel: unlock, search and fast intake without the main window.
//! It renders state owned by the kept `MeApp`; nothing here touches the vault.
//! It stays open when ME. loses focus, so files can be dragged in from Finder;
//! Esc, the status item or "Open ME." close it.

use gpui::{
    App, AppContext, Bounds, Context, Entity, ExternalPaths, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyBinding, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, WindowBounds, WindowHandle,
    WindowKind, WindowOptions, actions, div, point, prelude::FluentBuilder, px, rgb, size,
};

use crate::assets::icon;
use crate::input::TextInput;
use crate::lifecycle;
use crate::shell::MeApp;
use crate::shell::quick_api::{QuickAccess, QuickPending};
use crate::status_item::Anchor;
use crate::theme::*;

actions!(quick_panel, [Submit, Close]);

pub fn register_bindings(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", Submit, Some("QuickPanel")),
        KeyBinding::new("escape", Close, Some("QuickPanel")),
    ]);
}

/// Opens the panel below the status item, or closes it when already open.
pub fn toggle(anchor: Option<Anchor>, cx: &mut App) {
    if let Some(panel) = panel_window(cx) {
        let _ = panel.update(cx, |_, window, _| window.remove_window());
        return;
    }
    let app = lifecycle::app(cx);
    let panel = size(
        px(layout::QUICK_PANEL_WIDTH),
        px(layout::QUICK_PANEL_HEIGHT),
    );
    let display = anchor.and_then(|anchor| {
        cx.displays()
            .into_iter()
            .find(|display| u32::from(display.id()) == anchor.display)
            .map(|display| (anchor, display))
    });
    let (display_id, origin) = match display {
        Some((anchor, display)) => {
            let width = f64::from(display.bounds().size.width);
            let inset = f64::from(space::SM);
            let x = (anchor.center_x - f64::from(layout::QUICK_PANEL_WIDTH) / 2.)
                .min(width - f64::from(layout::QUICK_PANEL_WIDTH) - inset)
                .max(inset);
            (
                Some(display.id()),
                point(px(x as f32), px(anchor.bottom_y as f32 + space::XS)),
            )
        }
        None => (None, point(px(space::SECTION), px(space::SECTION))),
    };
    let opened = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin,
                size: panel,
            })),
            titlebar: None,
            focus: true,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: false,
            display_id,
            app_id: Some("me-desktop".into()),
            ..Default::default()
        },
        |_, cx| cx.new(|cx| QuickPanel::new(app, cx)),
    );
    if opened.is_err() {
        me_diagnostics::record("quick_panel_open_failed", &[]);
        return;
    }
    cx.activate(true);
}

fn panel_window(cx: &App) -> Option<WindowHandle<QuickPanel>> {
    cx.windows()
        .into_iter()
        .find_map(|window| window.downcast::<QuickPanel>())
}

pub struct QuickPanel {
    app: Entity<MeApp>,
    focus: FocusHandle,
    password: Entity<TextInput>,
    query: Entity<TextInput>,
    note: Entity<TextInput>,
    /// Which field received focus for the current access state.
    focused_for: Option<bool>,
    note_sent: bool,
    _subscriptions: Vec<Subscription>,
}

impl QuickPanel {
    fn new(app: Entity<MeApp>, cx: &mut Context<Self>) -> Self {
        let password = cx.new(TextInput::password);
        let query = cx.new(|cx| TextInput::new("Search your details…", cx));
        let note = cx.new(|cx| TextInput::multiline("Type or paste a note…", cx));
        let subscriptions = vec![
            // Locking elsewhere clears everything this panel shows or holds.
            cx.observe(&app, |this, app, cx| {
                if !matches!(app.read(cx).quick_access(), QuickAccess::Ready) {
                    for field in [&this.query, &this.note] {
                        field.update(cx, |input, cx| input.set_text("", cx));
                    }
                }
                cx.notify();
            }),
            cx.observe(&query, |_, _, cx| cx.notify()),
        ];
        Self {
            app,
            focus: cx.focus_handle(),
            password,
            query,
            note,
            focused_for: None,
            note_sent: false,
            _subscriptions: subscriptions,
        }
    }

    fn submit(&mut self, _: &Submit, window: &mut Window, cx: &mut Context<Self>) {
        match self.app.read(cx).quick_access() {
            QuickAccess::Locked { busy: false, .. } => self.unlock(cx),
            QuickAccess::Ready if self.query.focus_handle(cx).is_focused(window) => {
                let first = self
                    .app
                    .read(cx)
                    .quick_results(&self.query.read(cx).content)
                    .into_iter()
                    .next();
                if let Some(fact) = first {
                    self.app.update(cx, |app, cx| app.quick_copy(fact, cx));
                }
            }
            _ => {}
        }
    }

    fn close(&mut self, _: &Close, window: &mut Window, _: &mut Context<Self>) {
        window.remove_window();
    }

    fn unlock(&mut self, cx: &mut Context<Self>) {
        let password = zeroize::Zeroizing::new(self.password.read(cx).content.to_string());
        self.password.update(cx, |input, cx| input.set_text("", cx));
        self.app
            .update(cx, |app, cx| app.quick_unlock(&password, cx));
    }

    fn save_note(&mut self, cx: &mut Context<Self>) {
        let text = zeroize::Zeroizing::new(self.note.read(cx).content.to_string());
        if self.app.update(cx, |app, cx| app.quick_note(&text, cx)) {
            self.note.update(cx, |input, cx| input.set_text("", cx));
            self.note_sent = true;
        }
    }

    fn open_main(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.remove_window();
        lifecycle::open_main_window(cx);
    }

    fn field(
        &self,
        id: &'static str,
        input: &Entity<TextInput>,
        leading: Option<Icon>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let focus = input.focus_handle(cx);
        div()
            .id(id)
            .h(px(layout::CONTROL_LARGE))
            .px(px(space::MD))
            .rounded(px(radius::STANDARD))
            .border_1()
            .border_color(rgb(LINE))
            .bg(rgb(SURFACE))
            .flex()
            .items_center()
            .gap(px(space::SM))
            .on_click(move |_, window, _| window.focus(&focus))
            .when_some(leading, |s, name| {
                s.child(icon(name, IconSize::Medium, MUTED))
            })
            .child(div().flex_1().type_style(Type::Body).child(input.clone()))
    }

    fn locked(&self, busy: bool, error: Option<String>, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap(px(space::LG))
            .child(
                div()
                    .type_style(Type::Body)
                    .text_color(rgb(MUTED))
                    .child("Enter your master password to unlock your space."),
            )
            .child(self.field("quick-password", &self.password, None, cx))
            .when_some(error, |s, error| s.child(notice(error, true)))
            .child(
                primary_action()
                    .id("quick-unlock")
                    .h(px(layout::CONTROL_LARGE))
                    .hover(|s| s.bg(rgb(PRIMARY_HOVER)))
                    .on_click(cx.listener(|this, _, _, cx| this.unlock(cx)))
                    .child(if busy { "Unlocking…" } else { "Unlock" })
                    .child(action_indicator(busy, false)),
            )
    }

    fn setup(&self, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap(px(space::LG))
            .child(
                div()
                    .type_style(Type::Body)
                    .text_color(rgb(MUTED))
                    .child("Finish setting up ME. in its window first."),
            )
            .child(
                primary_action()
                    .id("quick-open-setup")
                    .hover(|s| s.bg(rgb(PRIMARY_HOVER)))
                    .on_click(cx.listener(|this, _, window, cx| this.open_main(window, cx)))
                    .child("Open ME.")
                    .child(icon(Icon::Arrow, IconSize::Medium, SURFACE)),
            )
    }

    fn ready(&self, cx: &mut Context<Self>) -> gpui::Div {
        let app = self.app.read(cx);
        let query = self.query.read(cx).content.clone();
        let results = app.quick_results(&query);
        let copied = app.quick_copied().map(str::to_owned);
        let pending = app.quick_pending();
        let intake_busy = app.quick_intake_busy();
        let (message, error) = app.quick_feedback();
        div()
            .flex()
            .flex_col()
            .gap(px(space::MD))
            .child(self.field("quick-search", &self.query, Some(Icon::Search), cx))
            .when(!query.trim().is_empty(), |s| {
                s.child(
                    div()
                        .flex()
                        .flex_col()
                        .when(results.is_empty(), |s| {
                            s.child(
                                div()
                                    .py(px(space::SM))
                                    .type_style(Type::Small)
                                    .text_color(rgb(MUTED))
                                    .child("Nothing matches yet."),
                            )
                        })
                        .children(results.into_iter().map(|fact| {
                            let is_copied = copied.as_deref() == Some(fact.id.as_str());
                            let id = SharedString::from(format!("quick-result-{}", fact.id));
                            let label = fact.label.clone();
                            let value = fact.value.clone();
                            div()
                                .id(id)
                                .px(px(space::MD))
                                .py(px(space::SM))
                                .rounded(px(radius::STANDARD))
                                .cursor_pointer()
                                .hover(|s| s.bg(rgb(HOVER)))
                                .flex()
                                .items_center()
                                .gap(px(space::SM))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let fact = fact.clone();
                                    this.app.update(cx, |app, cx| app.quick_copy(fact, cx));
                                }))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .child(
                                            div()
                                                .type_style(Type::Caption)
                                                .text_color(rgb(MUTED))
                                                .truncate()
                                                .child(label),
                                        )
                                        .child(
                                            div()
                                                .type_style(Type::Small)
                                                .font_family(font::MONO)
                                                .truncate()
                                                .child(value),
                                        ),
                                )
                                .child(
                                    div()
                                        .type_style(Type::Caption)
                                        .text_color(rgb(if is_copied { SUCCESS } else { FAINT }))
                                        .child(if is_copied { "Copied" } else { "Copy" }),
                                )
                        })),
                )
            })
            .child(self.drop_zone(pending, intake_busy, cx))
            .child(
                div()
                    .min_h(px(layout::CONTROL_LARGE))
                    .px(px(space::MD))
                    .py(px(space::SM))
                    .rounded(px(radius::STANDARD))
                    .border_1()
                    .border_color(rgb(LINE))
                    .bg(rgb(SURFACE))
                    .type_style(Type::Body)
                    .child(self.note.clone()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::SM))
                    .child(
                        div()
                            .flex_1()
                            .type_style(Type::Caption)
                            .text_color(rgb(MUTED))
                            .when(self.note_sent, |s| {
                                s.when_some(message, |s, message| s.child(message))
                            }),
                    )
                    .child(
                        secondary_action()
                            .id("quick-save-note")
                            .hover(|s| s.bg(rgb(HOVER)))
                            .on_click(cx.listener(|this, _, _, cx| this.save_note(cx)))
                            .child("Save note"),
                    ),
            )
            .when_some(error, |s, error| s.child(notice(error, true)))
    }

    fn drop_zone(
        &self,
        pending: Vec<QuickPending>,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let zone = div()
            .id("quick-drop")
            .min_h(px(layout::QUICK_DROP_HEIGHT))
            .p(px(space::MD))
            .rounded(px(radius::STANDARD))
            .border_1()
            .border_dashed()
            .border_color(rgb(DECORATIVE))
            .bg(rgb(BG))
            .flex()
            .flex_col()
            .gap(px(space::SM))
            .justify_center()
            .drag_over::<ExternalPaths>(|s, _, _, _| s.border_color(rgb(ACCENT)).bg(rgb(HOVER)))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.app
                    .update(cx, |app, cx| app.quick_drop(paths.paths(), cx));
            }));
        if pending.is_empty() {
            return zone.items_center().child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::SM))
                    .text_color(rgb(MUTED))
                    .child(icon(Icon::Upload, IconSize::Medium, MUTED))
                    .child(div().type_style(Type::Small).child(if busy {
                        "Reading dropped files…"
                    } else {
                        "Drop documents here"
                    })),
            );
        }
        let ready = pending.iter().filter(|file| file.error.is_none()).count();
        zone.children(pending.into_iter().map(|file| {
            div()
                .flex()
                .items_center()
                .gap(px(space::SM))
                .child(icon(Icon::Document, IconSize::Small, MUTED))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .type_style(Type::Small)
                        .truncate()
                        .child(file.name),
                )
                .child(
                    div()
                        .type_style(Type::Caption)
                        .text_color(rgb(if file.error.is_some() { DANGER } else { FAINT }))
                        .child(file.error.unwrap_or(file.detail)),
                )
        }))
        .child(
            div()
                .flex()
                .justify_end()
                .gap(px(space::SM))
                .child(
                    secondary_action()
                        .id("quick-drop-cancel")
                        .h(px(layout::CONTROL_COMPACT))
                        .hover(|s| s.bg(rgb(HOVER)))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.app.update(cx, |app, cx| app.quick_cancel_drop(cx))
                        }))
                        .child("Cancel"),
                )
                .when(ready > 0, |s| {
                    s.child(
                        primary_action()
                            .id("quick-drop-add")
                            .h(px(layout::CONTROL_COMPACT))
                            .hover(|s| s.bg(rgb(PRIMARY_HOVER)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.app.update(cx, |app, cx| app.quick_confirm_drop(cx))
                            }))
                            .child(if ready == 1 {
                                "Add 1 file".to_string()
                            } else {
                                format!("Add {ready} files")
                            }),
                    )
                }),
        )
    }
}

fn notice(message: String, error: bool) -> impl IntoElement {
    div()
        .px(px(space::MD))
        .py(px(space::SM))
        .rounded(px(radius::STANDARD))
        .bg(rgb(if error {
            DANGER_SURFACE
        } else {
            WARNING_SURFACE
        }))
        .text_color(rgb(if error { DANGER } else { WARNING }))
        .type_style(Type::Small)
        .child(message)
}

impl Focusable for QuickPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for QuickPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let access = self.app.read(cx).quick_access();
        let ready = matches!(access, QuickAccess::Ready);
        let locked = matches!(access, QuickAccess::Locked { .. });
        // Focus the field that fits the state once, when that state appears.
        if (locked || ready) && self.focused_for != Some(ready) {
            self.focused_for = Some(ready);
            let target = if ready { &self.query } else { &self.password };
            window.focus(&target.focus_handle(cx));
        }
        let body = match access {
            QuickAccess::Setup => self.setup(cx),
            QuickAccess::Locked { busy, error } => self.locked(busy, error, cx),
            QuickAccess::Ready => self.ready(cx),
        };
        div()
            .key_context("QuickPanel")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::close))
            .size_full()
            .p(px(space::XXL))
            .flex()
            .flex_col()
            .gap(px(space::LG))
            .bg(rgb(SURFACE))
            .border_1()
            .border_color(rgb(LINE))
            .rounded(px(radius::STANDARD))
            .font_family(font::SANS)
            .text_color(rgb(INK))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(wordmark(true))
                    .child(
                        div()
                            .id("quick-open-main")
                            .type_style(Type::Small)
                            .text_color(rgb(ACCENT))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, window, cx| this.open_main(window, cx)))
                            .child("Open ME."),
                    ),
            )
            .child(body)
    }
}
