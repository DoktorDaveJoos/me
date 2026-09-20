//! Synthetic wipe regression and visual harness. Never opens a user's vault.
//! ME_VAULT_DIR=/tmp/<fresh-folder>/vault cargo run -p me-app --example wipe_gallery -- [settings|confirm|progress|error|verify] [small]
#![allow(dead_code)]
#[path = "../src/assets.rs"]
mod assets;
#[path = "../src/design_system.rs"]
mod design_system;
#[path = "../src/input.rs"]
mod input;
#[path = "../src/theme.rs"]
mod theme;
#[cfg(any(debug_assertions, feature = "development-tools"))]
mod shell {
    include!("../src/shell.rs");
    pub fn fixture(cx: &mut Context<MeApp>, vault: Vault, mode: &str) -> MeApp {
        let mut app = MeApp::new(cx);
        app.codex_cancel = Some(Arc::new(std::sync::atomic::AtomicBool::new(true)));
        app.codex_ready = true;
        app.unlocked = true;
        app.initialized = true;
        app.busy = false;
        app.focus_filter_on_ready = false;
        app.show_settings = true;
        app.settings = vault.settings().unwrap();
        app.collection = vault.collection("", false).unwrap();
        app.import_jobs = vault.import_jobs().unwrap();
        app.session = Arc::new(Mutex::new(Some(vault)));
        app.development.confirming = mode == "confirm";
        app.development.wiping = mode == "progress";
        if mode == "error" {
            app.error = Some("Database cleared, but some stored files could not be removed. Check permissions and choose Wipe data again.".into());
        }
        app
    }
    pub fn verify(view: &Entity<MeApp>, cx: &mut App) {
        view.update(cx, |_, cx| {
            cx.spawn(async move |this, cx| {
                let executor = cx.background_executor().clone();
                executor.timer(std::time::Duration::from_millis(100)).await;
                let (old, cancel) = this.update(cx, |this, cx| {
                    let old = this.session.clone();
                    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
                    this.active_imports.insert(2, ActiveImport::new(cancel.clone(), true));
                    this.attachments.push(2);
                    this.selected_reviews.insert("synthetic-review".into());
                    this.wipe_data(cx);
                    assert!(!this.development.wiping, "Unconfirmed wipe must do nothing");
                    this.development.confirming = true;
                    this.wipe_data(cx);
                    assert!(this.development.wiping && !this.app_ready());
                    assert!(this.active_imports.is_empty() && this.attachments.is_empty());
                    assert!(this.selected_reviews.is_empty());
                    assert!(!Arc::ptr_eq(&old, &this.session));
                    (old, cancel)
                }).unwrap();
                for _ in 0..1000 {
                    if this.update(cx, |this, _| !this.development.wiping).unwrap() { break; }
                    executor.timer(std::time::Duration::from_millis(10)).await;
                }
                assert!(cancel.load(std::sync::atomic::Ordering::SeqCst));
                assert!(executor.spawn(async move { old.lock().unwrap().is_none() }).await, "Late workers must lose vault access");
                this.update(cx, |this, cx| {
                    assert!(this.app_ready() && !this.busy && this.codex_ready);
                    assert!(this.error.is_none(), "{:?}", this.error);
                    assert!(this.collection.items.is_empty() && this.import_jobs.is_empty());
                    assert!(!this.settings.automatic_evaluation);
                    assert!(this.notice.as_ref().unwrap().contains("Data wiped"));
                    this.run_change(cx, |vault| {
                        vault.save_note(None, "After reset", "Still writable")?;
                        Ok("Saved after reset".into())
                    });
                }).unwrap();
                for _ in 0..1000 {
                    if this.update(cx, |this, _| !this.busy).unwrap() { break; }
                    executor.timer(std::time::Duration::from_millis(10)).await;
                }
                this.update(cx, |this, _| {
                    assert!(this.error.is_none());
                    assert_eq!(this.collection.items.len(), 1);
                }).unwrap();
                println!("PASS wipe: confirmation required; workers canceled and disconnected; UI cleared; vault stays unlocked; settings and connection preserved; writes succeed afterward.");
                cx.update(|cx| cx.quit()).unwrap();
            }).detach();
        });
    }
}
#[cfg(any(debug_assertions, feature = "development-tools"))]
fn main() {
    use gpui::{
        App, Application, Bounds, KeyBinding, WindowBounds, WindowOptions, prelude::*, px, size,
    };
    let root = std::path::PathBuf::from(
        std::env::var_os("ME_VAULT_DIR").expect("Set a fresh synthetic ME_VAULT_DIR"),
    );
    assert!(root.starts_with("/tmp") || root.starts_with("/private/tmp"));
    let parent = root.parent().unwrap();
    std::fs::create_dir_all(parent).unwrap();
    let mut vault = me_core::Vault::create(&root, "synthetic-wipe-password").unwrap();
    vault.set_automatic_evaluation(false).unwrap();
    vault
        .save_note(None, "Synthetic note", "Synthetic value")
        .unwrap();
    let file = parent.join("original.txt");
    std::fs::write(&file, "SYNTHETIC FILE").unwrap();
    vault
        .import_document(
            &file,
            "synthetic-delivery",
            me_core::DocumentClass::Personal,
        )
        .unwrap();
    let mode = std::env::args().nth(1).unwrap_or_else(|| "settings".into());
    let small = std::env::args().any(|arg| arg == "small");
    Application::new()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            assets::load_fonts(cx).unwrap();
            input::register_bindings(cx);
            cx.bind_keys([
                KeyBinding::new("escape", shell::Dismiss, Some("Me")),
                KeyBinding::new("cmd-,", shell::OpenSettings, Some("Me")),
                KeyBinding::new("cmd-shift-l", shell::LockVault, Some("Me")),
            ]);
            let dimensions = size(
                px(if small { 800. } else { 1120. }),
                px(if small { 600. } else { 820. }),
            );
            let verify = mode == "verify";
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                            None, dimensions, cx,
                        ))),
                        ..Default::default()
                    },
                    move |_, cx| cx.new(|cx| shell::fixture(cx, vault, &mode)),
                )
                .unwrap();
            window
                .update(cx, |_, window, _| {
                    window.resize(dimensions);
                    window.set_window_title("ME Wipe — Synthetic");
                })
                .unwrap();
            if verify {
                shell::verify(&window.entity(cx).unwrap(), cx);
            }
            cx.activate(true);
        });
}
#[cfg(not(any(debug_assertions, feature = "development-tools")))]
fn main() {}
