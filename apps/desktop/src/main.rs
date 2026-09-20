#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("ME. supports macOS and Linux.");

mod assets;
mod design_system;
mod input;
mod shell;
mod theme;

use gpui::{
    App, Application, Bounds, Focusable, KeyBinding, Menu, MenuItem, OsAction, TitlebarOptions,
    WindowBounds, WindowOptions, actions, point, prelude::*, px, size,
};
use me_core::APP_NAME;
use shell::{
    AddFact, Confirm, Dismiss, FocusSearch, LockVault, MeApp, NextField, OpenSettings,
    PreviousField,
};

actions!(me, [Quit]);

fn main() {
    if let Some(root) =
        me_agent::default_vault_path().and_then(|p| p.parent().map(|p| p.join("logs")))
    {
        me_diagnostics::init(root, env!("ME_BUILD_ID"));
    }
    Application::new()
        .with_assets(assets::Assets)
        .run(|cx: &mut App| {
            assets::load_fonts(cx).expect("ME. could not load its bundled Geist fonts");
            input::register_bindings(cx);
            cx.on_action(|_: &Quit, cx| cx.quit());
            let modifier = if cfg!(target_os = "macos") {
                "cmd"
            } else {
                "ctrl"
            };
            cx.bind_keys([
                KeyBinding::new(&format!("{modifier}-q"), Quit, None),
                KeyBinding::new(&format!("{modifier}-,"), OpenSettings, Some("Me")),
                KeyBinding::new(&format!("{modifier}-k"), FocusSearch, Some("Me")),
                KeyBinding::new(&format!("{modifier}-n"), AddFact, Some("Me")),
                KeyBinding::new(&format!("{modifier}-shift-l"), LockVault, Some("Me")),
                KeyBinding::new("escape", Dismiss, Some("Me")),
                KeyBinding::new("enter", Confirm, Some("Me")),
                KeyBinding::new("tab", NextField, Some("Me")),
                KeyBinding::new("shift-tab", PreviousField, Some("Me")),
            ]);
            cx.set_menus(vec![
                Menu {
                    name: APP_NAME.into(),
                    items: vec![
                        MenuItem::action("Settings…", OpenSettings),
                        MenuItem::Separator,
                        MenuItem::action("Quit ME.", Quit),
                    ],
                },
                Menu {
                    name: "Edit".into(),
                    items: vec![
                        MenuItem::os_action("Cut", input::Cut, OsAction::Cut),
                        MenuItem::os_action("Copy", input::Copy, OsAction::Copy),
                        MenuItem::os_action("Paste", input::Paste, OsAction::Paste),
                        MenuItem::Separator,
                        MenuItem::os_action("Select All", input::SelectAll, OsAction::SelectAll),
                    ],
                },
            ]);
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let bounds = Bounds::centered(None, size(px(1120.), px(820.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(800.), px(600.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some(APP_NAME.into()),
                        appears_transparent: true,
                        traffic_light_position: Some(point(px(20.), px(20.))),
                    }),
                    app_id: Some("me-desktop".into()),
                    ..Default::default()
                },
                |window, cx| {
                    let view = cx.new(MeApp::new);
                    window.focus(&view.focus_handle(cx));
                    view
                },
            )
            .expect("ME. could not create its main window");
            cx.activate(true);
        });
}
