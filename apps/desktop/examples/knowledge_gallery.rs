//! Native knowledge-map smoke harness. Requires a fresh synthetic vault under /tmp.
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
    pub fn fixture(cx: &mut Context<MeApp>, mut vault: Vault, mode: String) -> MeApp {
        let mut app = MeApp::new(cx);
        app.codex_cancel = Some(Arc::new(std::sync::atomic::AtomicBool::new(true)));
        app.codex_ready = false;
        app.unlocked = true;
        app.initialized = true;
        app.busy = false;
        app.root = None;
        app.focus_filter_on_ready = false;
        app.settings.automatic_evaluation = false;
        app.collection = vault.collection("", false).unwrap();
        app.knowledge.install(vault.knowledge_map().unwrap());
        if mode == "error" {
            app.knowledge.error =
                Some("Synthetic loading failure. Choose Refresh to try again.".into());
        }
        if mode == "loading" {
            app.knowledge.loading = true;
        }
        app.page = Page::Knowledge;
        app.refresh_knowledge_routes(cx);
        app.session = Arc::new(Mutex::new(Some(vault)));
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
    let mode = std::env::args().nth(1).unwrap_or_default();
    if !matches!(mode.as_str(), "empty" | "loading" | "error") {
        for (label, value) in [
            ("Preferred name", "Alex"),
            ("Languages", "English, German"),
            ("Home city", "Berlin"),
            ("Contact preference", "Email"),
            ("Reading", "Science fiction"),
            ("Working hours", "09:00–17:00"),
        ] {
            vault.save_note(None, label, value).unwrap();
        }
        for (filename, folder, fields, accept) in [
            (
                "Employment",
                "Work",
                vec![
                    ("document.Employer", "Northstar Studio"),
                    ("document.Role", "Product designer"),
                    ("document.Hours", "32 hours / week"),
                    ("document.Start date", "1 September 2026"),
                    ("document.Location", "Berlin"),
                ],
                true,
            ),
            (
                "Home insurance",
                "Home",
                vec![
                    ("document.Insurer", "Evergreen"),
                    ("document.Policy", "SYN-2048-AB"),
                    ("document.Renewal", "1 January 2027"),
                    ("document.Coverage", "Contents"),
                ],
                true,
            ),
            (
                "Membership",
                "Personal",
                vec![
                    ("document.Organization", "City Library"),
                    ("document.Member ID", "SYN-01824"),
                ],
                false,
            ),
        ] {
            let path = parent.join(format!("{filename}.txt"));
            let text = format!(
                "SYNTHETIC EXAMPLE\nAlex Morgan\n{}",
                fields
                    .iter()
                    .map(|(label, value)| format!("{label}: {value}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            std::fs::write(&path, text).unwrap();
            let item = vault
                .import_document(&path, filename, me_core::DocumentClass::Personal)
                .unwrap();
            vault.begin_evaluation(item, false).unwrap();
            let input = vault.prepare_extraction(item, "synthetic").unwrap();
            let facts = fields
                .into_iter()
                .map(|(property, value)| me_core::ExtractedFact {
                    property: property.into(),
                    value: value.into(),
                    quote: value.into(),
                    subject_quote: "Alex Morgan".into(),
                    context_quote: String::new(),
                    segment_id: input.segments[0].segment_id.clone(),
                })
                .collect();
            vault
                .finish_extraction(&input, me_core::ExtractionOutput { facts })
                .unwrap();
            vault.finish_evaluation(item, None).unwrap();
            if accept {
                vault.review_proposals(item, true).unwrap();
            }
            vault
                .save_document_folders(
                    &[me_core::FolderAssignment {
                        item,
                        path: vec![folder.into()],
                    }],
                    &[item],
                )
                .unwrap();
        }
    }
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
                    move |_, cx| cx.new(|cx| shell::fixture(cx, vault, mode)),
                )
                .unwrap();
            window
                .update(cx, |_, window, _| {
                    window.resize(dimensions);
                    window.set_window_title("ME Knowledge Gallery — Synthetic");
                })
                .unwrap();
            cx.activate(true);
        });
}
