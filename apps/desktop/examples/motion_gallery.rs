//! Native motion smoke harness. Only fresh synthetic vaults under /tmp are allowed.
//! Modes: locked, search, settings, dialog, knowledge, imports; optional small, reduced, busy.
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
    pub fn toggle_activity(view: &Entity<MeApp>, cx: &mut App) {
        view.update(cx, |this, cx| {
            this.busy = !this.busy;
            cx.notify();
        });
    }
    pub fn fixture(cx: &mut Context<MeApp>, mut vault: Vault, mode: String) -> MeApp {
        let mut app = MeApp::new(cx);
        app.codex_cancel = Some(Arc::new(std::sync::atomic::AtomicBool::new(true)));
        app.codex_ready = false;
        app.unlocked = true;
        app.initialized = true;
        app.busy = std::env::args().any(|arg| arg == "busy");
        app.root = if mode == "locked" {
            MeApp::vault_path()
        } else {
            None
        };
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
        app.page = match mode.as_str() {
            "knowledge" => Page::Knowledge,
            "imports" => Page::Imports,
            _ => Page::Search,
        };
        app.show_settings = mode == "settings";
        app.show_add = mode == "dialog";
        if mode == "locked" {
            app.unlocked = false;
            app.account.mode = AccountMode::Unlock;
            app.focus_password_on_ready = true;
        }
        app.refresh_knowledge_routes(cx);
        app.session = Arc::new(Mutex::new(if app.unlocked {
            Some(vault)
        } else {
            // The real locked screen owns no vault lock before authentication.
            drop(vault);
            None
        }));
        app
    }
}
use gpui::{
    App, Application, Bounds, KeyBinding, WindowBounds, WindowOptions, prelude::*, px, size,
};
gpui::actions!(motion_gallery, [TogglePreviewSize, TogglePreviewActivity]);
struct Gallery {
    app: gpui::Entity<shell::MeApp>,
    drawing_phase: Option<f32>,
}
impl Render for Gallery {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        use theme::*;
        if let Some(phase) = self.drawing_phase {
            return gpui::div()
                .id("drawing-phases")
                .size_full()
                .relative()
                .bg(gpui::rgb(BG))
                .font_family(font::SANS)
                .text_color(gpui::rgb(INK))
                .type_style(Type::Body)
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(space::XXL))
                .child(motion::field_at(motion::Field::Identity, phase))
                .child(onboarding_fingerprint_at(phase))
                .child(heading("Identity drawing & orbit"))
                .child(eyebrow(format!(
                    "Paused at {:.0}% · click to advance",
                    phase * 100.
                )))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.drawing_phase = Some(if this.drawing_phase.unwrap() < 0.9 {
                        this.drawing_phase.unwrap() + 0.2
                    } else {
                        0.1
                    });
                    cx.notify();
                }))
                .into_any_element();
        }
        self.app.clone().into_any_element()
    }
}
fn main() {
    let root = std::env::var_os("ME_VAULT_DIR")
        .map(std::path::PathBuf::from)
        .expect("Set ME_VAULT_DIR to a fresh synthetic vault path");
    assert!(root.starts_with("/tmp") || root.starts_with("/private/tmp"));
    let parent = root.parent().unwrap();
    std::fs::create_dir_all(parent).unwrap();
    if std::env::args().any(|arg| arg == "reduced") {
        std::fs::write(
            parent.join("interface.json"),
            r#"{"version":1,"reduce_motion":true}"#,
        )
        .unwrap();
    }
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
                KeyBinding::new("f6", TogglePreviewSize, None),
                KeyBinding::new("f8", TogglePreviewActivity, None),
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
                    move |_, cx| {
                        let drawing_phase = (mode == "drawing").then_some(0.3);
                        let app = cx.new(|cx| shell::fixture(cx, vault, mode));
                        cx.new(|_| Gallery { app, drawing_phase })
                    },
                )
                .unwrap();
            window
                .update(cx, |_, window, _| {
                    window.resize(dimensions);
                    window.set_window_title("ME Motion Gallery — Synthetic");
                    eprintln!("Synthetic preview viewport: {:?}", window.viewport_size());
                })
                .unwrap();
            let minimum = std::cell::Cell::new(small);
            cx.on_action(move |_: &TogglePreviewSize, cx| {
                minimum.set(!minimum.get());
                let small = minimum.get();
                cx.defer(move |cx| {
                    let _ = window.update(cx, |_, window, _| {
                        window.resize(size(
                            px(if small { 800. } else { 1120. }),
                            px(if small { 600. } else { 820. }),
                        ))
                    });
                });
            });
            cx.on_action(move |_: &TogglePreviewActivity, cx| {
                cx.defer(move |cx| {
                    let _ = window.update(cx, |gallery, _, cx| {
                        shell::toggle_activity(&gallery.app, cx)
                    });
                });
            });
            cx.activate(true);
        });
}
