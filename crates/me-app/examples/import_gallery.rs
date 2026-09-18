//! Synthetic import UI harness; no user vault or provider connection is used.
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
    pub fn fixture(
        cx: &mut Context<MeApp>,
        vault: Vault,
        samples: Vec<PathBuf>,
        mode: String,
    ) -> MeApp {
        let mut app = MeApp::new(cx);
        // Prevent the deferred connection check from launching a provider process.
        app.codex_cancel = Some(Arc::new(std::sync::atomic::AtomicBool::new(true)));
        app.codex_ready = true;
        app.unlocked = true;
        app.initialized = true;
        app.busy = false;
        app.root = None;
        app.focus_filter_on_ready = false;
        app.page = Page::Imports;
        app.settings.automatic_evaluation = false;
        app.collection = vault.collection("", false).unwrap();
        app.import_jobs = vault.import_jobs().unwrap();
        app.session = Arc::new(Mutex::new(Some(vault)));
        if mode == "confirm" {
            app.accept_documents(&samples, cx);
        }
        if mode == "progress" {
            for (index, job) in app.import_jobs.iter().take(2).enumerate() {
                let mut active =
                    ActiveImport::new(Arc::new(std::sync::atomic::AtomicBool::new(false)), true);
                active.stage = if index == 0 {
                    me_core::ImportStage::Normalizing
                } else {
                    me_core::ImportStage::Verifying
                };
                active.current = if index == 0 { 3 } else { 2 };
                active.total = if index == 0 { 8 } else { 4 };
                active.message = if index == 0 {
                    "Recognizing text and preserving page evidence…"
                } else {
                    "Checking sources and looking for missing details…"
                }
                .into();
                app.active_imports.insert(job.item, active);
            }
        }
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
    assert!(
        root.starts_with("/tmp") || root.starts_with("/private/tmp"),
        "Only synthetic /tmp vaults are allowed"
    );
    let parent = root.parent().unwrap();
    std::fs::create_dir_all(parent).unwrap();
    let mut vault = me_core::Vault::create(&root, "synthetic-gallery-passphrase").unwrap();
    vault.set_automatic_evaluation(false).unwrap();
    let names = [
        "Health insurance · September.eml",
        "Letter from the insurance provider with a long descriptive filename.pdf",
        "Personal notes.txt",
    ];
    let mut samples = Vec::new();
    for name in names {
        let path = parent.join(name);
        std::fs::write(&path, "SYNTHETIC EXAMPLE\nReference: 00042").unwrap();
        vault
            .import_document(&path, name, me_core::DocumentClass::Unclassified)
            .unwrap();
        samples.push(path);
    }
    let mode = std::env::args().nth(1).unwrap_or_default();
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
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                            None,
                            size(
                                px(if small { 800. } else { 1120. }),
                                px(if small { 600. } else { 780. }),
                            ),
                            cx,
                        ))),
                        ..Default::default()
                    },
                    move |_, cx| cx.new(|cx| shell::fixture(cx, vault, samples, mode)),
                )
                .unwrap();
            window
                .update(cx, |_, window, _| {
                    window.set_window_title("ME Import Gallery — Synthetic")
                })
                .unwrap();
            cx.activate(true);
        });
}
