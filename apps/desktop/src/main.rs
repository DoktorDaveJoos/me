#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("ME. supports macOS and Linux.");

mod assets;
mod design_system;
mod input;
mod lifecycle;
#[cfg(target_os = "macos")]
mod quick_panel;
mod shell;
mod single_instance;
#[cfg(target_os = "macos")]
mod status_item;
mod theme;

use futures_util::StreamExt;
use gpui::{App, Application, KeyBinding, Menu, MenuItem, OsAction, actions};
use me_core::APP_NAME;
use shell::{
    AddFact, Confirm, Dismiss, FocusSearch, LockVault, NextField, OpenSettings, PreviousField,
};

actions!(me, [Quit]);

fn main() {
    if let Some(root) =
        me_agent::default_vault_path().and_then(|p| p.parent().map(|p| p.join("logs")))
    {
        me_diagnostics::init(root, env!("ME_BUILD_ID"));
    }
    let listener = match me_agent::default_vault_path()
        .map(|vault| single_instance::claim(&single_instance::socket_path(&vault)))
    {
        Some(Ok(single_instance::Claim::HandedOff)) => return,
        Some(Ok(single_instance::Claim::Primary(listener))) => Some(listener),
        Some(Err(_)) => {
            me_diagnostics::record("single_instance_unavailable", &[]);
            None
        }
        None => None,
    };
    let app = Application::new().with_assets(assets::Assets);
    app.on_reopen(lifecycle::open_main_window);
    app.run(move |cx: &mut App| {
        assets::load_fonts(cx).expect("ME. could not load its bundled Geist fonts");
        input::register_bindings(cx);
        #[cfg(target_os = "macos")]
        quick_panel::register_bindings(cx);
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
            KeyBinding::new(
                &format!("{modifier}-shift-n"),
                shell::NewCredential,
                Some("Me"),
            ),
            KeyBinding::new(&format!("{modifier}-e"), shell::EditLogin, Some("Me")),
            KeyBinding::new(&format!("{modifier}-s"), shell::SaveLogin, Some("Me")),
            KeyBinding::new("down", shell::NextLogin, Some("LoginsList")),
            KeyBinding::new("up", shell::PreviousLogin, Some("LoginsList")),
            KeyBinding::new("tab", NextField, Some("Me")),
            KeyBinding::new("shift-tab", PreviousField, Some("Me")),
        ]);
        cx.set_menus(vec![
            Menu {
                name: APP_NAME.into(),
                items: vec![
                    MenuItem::action("Settings…", OpenSettings),
                    MenuItem::action("Privacy permissions…", shell::OpenPermissions),
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
        cx.on_window_closed(lifecycle::main_window_closed).detach();
        if let Some(listener) = listener {
            let (show, mut requests) = futures_channel::mpsc::unbounded();
            single_instance::serve(listener, show);
            cx.spawn(async move |cx| {
                while requests.next().await.is_some() {
                    if cx.update(lifecycle::open_main_window).is_err() {
                        break;
                    }
                }
            })
            .detach();
        }
        lifecycle::open_main_window(cx);
        #[cfg(target_os = "macos")]
        status_item::install(cx);
    });
}
