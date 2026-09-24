//! Isolated native login smoke harness. Uses only the checked-in synthetic 1PUX.
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
        window: &mut Window,
        vault: Vault,
        mode: String,
        fixture: PathBuf,
    ) -> MeApp {
        let mut app = MeApp::new(cx);
        app.codex_cancel = Some(Arc::new(std::sync::atomic::AtomicBool::new(true)));
        app.codex_ready = false;
        app.unlocked = true;
        app.initialized = true;
        app.busy = false;
        app.root = None;
        app.focus_filter_on_ready = false;
        app.logins.arrival = Some(std::time::Instant::now());
        app.page = Page::Logins;
        app.settings.automatic_evaluation = false;
        app.collection = vault.collection("", false).unwrap();
        app.logins.items = vault.logins().unwrap();
        app.logins.loaded = true;
        app.logins.visible = (0..app.logins.items.len()).collect();
        if let Some(item) = app.logins.items.first() {
            app.logins.selected = Some(item.id);
            app.logins.details = Some(vault.login_details(item.id).unwrap());
        }
        if matches!(mode.as_str(), "loading" | "detail-error") {
            app.logins.details = None;
            app.logins.detail_loading = mode == "loading";
            if mode == "detail-error" {
                app.logins.error = Some("Couldn't open this login. Try again.".into());
            }
        }
        app.session = Arc::new(Mutex::new(Some(vault)));
        if mode == "preview" {
            app.show_onepassword = true;
            app.prepare_onepassword(fixture, cx);
        }
        if matches!(
            mode.as_str(),
            "new" | "capture" | "api-draft" | "registration-draft" | "accessibility"
        ) {
            app.new_credential(&NewCredential, window, cx);
            if mode == "accessibility" {
                app.logins.intake.permission_source = Some(context_source::Source {
                    identity: serde_json::json!({"id":0,"pid":0,"bundle":"synthetic.invalid"}),
                    name: "Synthetic browser".into(),
                    title: "Synthetic registration page".into(),
                    preview: None,
                });
            }
            if mode == "capture" {
                app.logins.intake.capture = Some(credential_capture::Capture::from_text(
                    "Synthetic window".into(),
                    "Recovery codes\nABCD-EFGH\nIJKL-MNOP",
                    "https://mail.example.test/security?token=synthetic",
                ));
            }
            if mode == "registration-draft" {
                let mut capture = credential_capture::Capture::from_window(br#"{"source":"Synthetic registration window","nodes":[{"label":"Email"},{"label":"Password"},{"label":"Register"}],"url":"https://forge.example.test/register","registration":true,"fill_target":{"source":{"id":0,"pid":0,"bundle":"synthetic.invalid"},"page":"https://forge.example.test/register","fingerprint":"synthetic"}}"#).unwrap();
                capture
                    .prepare_registration("preview@example.test")
                    .unwrap();
                app.logins.intake.capture = Some(capture);
                app.begin_credential(me_core::CredentialKind::Login, window, cx);
            }
            if mode == "api-draft" {
                app.logins.intake.capture = Some(credential_capture::Capture::from_text(
                    "Synthetic window".into(),
                    "API token\ntoken: SYNTHETIC-API-TOKEN\nproject: Test project\nenvironment: Development",
                    "https://api.example.test/settings",
                ));
                app.begin_credential(me_core::CredentialKind::Api, window, cx);
            }
        }
        if mode == "permissions" {
            app.open_permissions(&OpenPermissions, window, cx);
        }
        if mode == "edit" {
            app.edit_login(&EditLogin, window, cx);
        }
        if mode == "no-match" {
            app.login_search
                .update(cx, |i, cx| i.set_text("no matching login", cx));
            app.filter_logins(cx);
        }
        if mode == "error" {
            app.show_onepassword = true;
            app.error = Some(
                "This 1Password file is incomplete or damaged. Export it again as 1PUX.".into(),
            );
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
        .expect("Set an isolated /tmp ME_VAULT_DIR");
    assert!(root.starts_with("/tmp") || root.starts_with("/private/tmp"));
    std::fs::create_dir_all(root.parent().unwrap()).unwrap();
    let reduced = std::env::args().any(|a| a == "reduced");
    std::fs::write(
        root.parent().unwrap().join("interface.json"),
        format!(r#"{{"reduce_motion":{reduced}}}"#),
    )
    .unwrap();
    let mode = std::env::args().nth(1).unwrap_or_else(|| "list".into());
    let small = std::env::args().any(|a| a == "small");
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/me-core/tests/fixtures/synthetic-logins.1pux");
    let mut vault = if me_core::Vault::exists(&root).unwrap_or(false) {
        me_core::Vault::unlock(&root, "synthetic-gallery-passphrase").unwrap()
    } else {
        me_core::Vault::create(&root, "synthetic-gallery-passphrase").unwrap()
    };
    vault.set_automatic_evaluation(false).unwrap();
    if !matches!(mode.as_str(), "empty" | "preview" | "error") {
        vault
            .import_onepassword(&me_core::OnePasswordImport::read(&fixture).unwrap())
            .unwrap();
    }
    if std::env::args().any(|a| a == "live-icons") {
        for (item, (title, website)) in vault.logins().unwrap().into_iter().zip([
            ("GitHub — synthetic account", "https://github.com/"),
            ("Google — synthetic account", "https://www.google.com/"),
        ]) {
            let details = vault.login_details(item.id).unwrap();
            vault
                .update_login(
                    item.id,
                    me_core::LoginUpdate {
                        revision: details.revision,
                        title: title.into(),
                        favorite: details.favorite,
                        fields: vec![(
                            "/overview/url".into(),
                            zeroize::Zeroizing::new(website.into()),
                        )],
                    },
                )
                .unwrap();
        }
    }
    Application::new()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            assets::load_fonts(cx).unwrap();
            input::register_bindings(cx);
            let modifier = if cfg!(target_os = "macos") {
                "cmd"
            } else {
                "ctrl"
            };
            cx.bind_keys([
                KeyBinding::new("escape", shell::Dismiss, Some("Me")),
                KeyBinding::new("enter", shell::Confirm, Some("Me")),
                KeyBinding::new("tab", shell::NextField, Some("Me")),
                KeyBinding::new("shift-tab", shell::PreviousField, Some("Me")),
                KeyBinding::new(
                    &format!("{modifier}-shift-n"),
                    shell::NewCredential,
                    Some("Me"),
                ),
                KeyBinding::new(&format!("{modifier}-e"), shell::EditLogin, Some("Me")),
                KeyBinding::new(&format!("{modifier}-s"), shell::SaveLogin, Some("Me")),
                KeyBinding::new(&format!("{modifier}-k"), shell::FocusSearch, Some("Me")),
                KeyBinding::new(&format!("{modifier}-shift-l"), shell::LockVault, Some("Me")),
                KeyBinding::new("down", shell::NextLogin, Some("LoginsList")),
                KeyBinding::new("up", shell::PreviousLogin, Some("LoginsList")),
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
                        window_min_size: Some(size(px(800.), px(600.))),
                        ..Default::default()
                    },
                    move |window, cx| cx.new(|cx| shell::fixture(cx, window, vault, mode, fixture)),
                )
                .unwrap();
            window
                .update(cx, |_, window, _| {
                    window.resize(dimensions);
                    window.set_window_title("ME Logins — Synthetic Preview");
                })
                .unwrap();
            cx.activate(true);
        });
}
