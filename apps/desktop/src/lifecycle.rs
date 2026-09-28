//! Main window lifecycle. The `MeApp` entity, and with it an unlocked vault
//! session, outlives its window so ME. keeps running from the menu bar.

use gpui::{
    App, AppContext, Bounds, Entity, Focusable, Global, TitlebarOptions, WindowBounds,
    WindowHandle, WindowOptions, point, px, size,
};
use me_core::APP_NAME;

use crate::shell::MeApp;

struct KeptApp(Entity<MeApp>);

impl Global for KeptApp {}

/// The one `MeApp`, created on first use.
pub fn app(cx: &mut App) -> Entity<MeApp> {
    if let Some(kept) = cx.try_global::<KeptApp>() {
        return kept.0.clone();
    }
    let app = cx.new(MeApp::new);
    cx.set_global(KeptApp(app.clone()));
    app
}

pub fn main_window(cx: &App) -> Option<WindowHandle<MeApp>> {
    cx.windows()
        .into_iter()
        .find_map(|window| window.downcast::<MeApp>())
}

/// Activates the main window, opening it again if it was closed.
pub fn open_main_window(cx: &mut App) {
    #[cfg(target_os = "macos")]
    crate::status_item::set_dock_visible(true);
    cx.activate(true);
    if let Some(window) = main_window(cx) {
        let _ = window.update(cx, |_, window, _| window.activate_window());
        return;
    }
    let view = app(cx);
    let bounds = Bounds::centered(None, size(px(1120.), px(820.)), cx);
    let opened = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(800.), px(600.))),
            titlebar: Some(TitlebarOptions {
                title: Some(window_title().into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(20.), px(20.))),
            }),
            app_id: Some("me-desktop".into()),
            ..Default::default()
        },
        |window, cx| {
            window.focus(&view.focus_handle(cx));
            view
        },
    );
    if opened.is_err() {
        me_diagnostics::record("main_window_open_failed", &[]);
    }
}

/// On macOS ME. stays in the menu bar and leaves the Dock without a main
/// window. Linux has no tray yet, so closing the last window still quits there.
pub fn main_window_closed(cx: &mut App) {
    if main_window(cx).is_some() {
        return;
    }
    #[cfg(target_os = "macos")]
    crate::status_item::set_dock_visible(false);
    #[cfg(not(target_os = "macos"))]
    cx.quit();
}

fn window_title() -> &'static str {
    match std::env::var("ME_BUILD_CHANNEL").as_deref() {
        Ok("dev") => "ME Dev",
        Ok("preview") => "ME Preview",
        _ => APP_NAME,
    }
}
