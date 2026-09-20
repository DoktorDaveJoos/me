//! Local search smoke harness. Use a fresh ME_VAULT_DIR under /tmp and a
//! synthetic.1pux fixture beside it; never point this at a personal export.
#![allow(dead_code)]
#[path = "../src/assets.rs"]
mod assets;
#[path = "../src/design_system.rs"]
mod design_system;
#[path = "../src/input.rs"]
mod input;
#[path = "../src/theme.rs"]
mod theme;
mod shell {
    include!("../src/shell.rs");
    pub fn fixture(cx: &mut Context<MeApp>, vault: Vault, query: String) -> MeApp {
        let mut app = MeApp::new(cx);
        app.codex_cancel = Some(Arc::new(std::sync::atomic::AtomicBool::new(true)));
        app.codex_ready = false;
        app.unlocked = true;
        app.initialized = true;
        app.busy = false;
        app.root = None;
        app.focus_filter_on_ready = true;
        app.settings.automatic_evaluation = false;
        app.collection = vault.collection("", false).unwrap();
        app.filter.facts = vault.data_facts().unwrap();
        app.session = Arc::new(Mutex::new(Some(vault)));
        app.filter_input
            .update(cx, |input, cx| input.set_text(&query, cx));
        app.filter_changed(cx);
        app
    }
}
use gpui::{
    App, Application, Bounds, KeyBinding, WindowBounds, WindowOptions, prelude::*, px, size,
};
fn main() {
    let root = std::env::var_os("ME_VAULT_DIR")
        .map(std::path::PathBuf::from)
        .expect("Set ME_VAULT_DIR to a fresh synthetic vault path");
    assert!(root.starts_with("/tmp") || root.starts_with("/private/tmp"));
    let parent = root.parent().unwrap();
    std::fs::create_dir_all(parent).unwrap();
    let mut vault = me_core::Vault::create(&root, "synthetic-gallery-passphrase").unwrap();
    vault.set_automatic_evaluation(false).unwrap();
    let import = me_core::OnePasswordImport::read(&parent.join("synthetic.1pux")).unwrap();
    vault.import_onepassword(&import).unwrap();
    vault
        .save_note(None, "Instagram reminder", "Synthetic reference")
        .unwrap();
    vault.save_note(None, "Steuer-ID", "01234567890").unwrap();
    let path = parent.join("Instagram guide with a long synthetic filename for search layout.txt");
    std::fs::write(&path, "SYNTHETIC EXAMPLE").unwrap();
    vault
        .import_document(&path, "synthetic", me_core::DocumentClass::Unclassified)
        .unwrap();
    let query = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Instagram".into());
    let small = std::env::args().any(|arg| arg == "small");
    Application::new()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            assets::load_fonts(cx).unwrap();
            input::register_bindings(cx);
            cx.bind_keys([
                KeyBinding::new("escape", shell::Dismiss, Some("Me")),
                KeyBinding::new("enter", shell::Confirm, Some("Me")),
            ]);
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let dimensions = size(
                px(if small { 800. } else { 1120. }),
                px(if small { 600. } else { 820. }),
            );
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                            None, dimensions, cx,
                        ))),
                        ..Default::default()
                    },
                    move |_, cx| cx.new(|cx| shell::fixture(cx, vault, query)),
                )
                .unwrap();
            window
                .update(cx, |_, window, _| {
                    window.resize(dimensions);
                    window.set_window_title("ME Search Gallery — Synthetic");
                })
                .unwrap();
            cx.activate(true);
        });
}
