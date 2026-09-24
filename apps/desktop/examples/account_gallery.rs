//! Synthetic account screens, using the production views. No network required.
//! ME_VAULT_DIR=/tmp/<fresh>/vault ME_CODEX_BIN=/usr/bin/false cargo run -p me-app --example account_gallery -- [create|signin|recover|code|error|busy|complete|unlock[-error|-busy|-long]|provider[-ready|-error|-working|-quota]] [small]
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
    pub fn cycle(view: &Entity<MeApp>, index: usize, cx: &mut App) {
        view.update(cx, |this, cx| {
            this.busy = false;
            this.error = None;
            this.account.pending = None;
            this.account.complete = None;
            this.unlocked = false;
            this.account.binding = None;
            this.codex_ready = false;
            this.account.mode = match index % 11 {
                0 => AccountMode::SignIn,
                1 => AccountMode::Recover,
                _ => AccountMode::Register,
            };
            if index % 11 == 2 {
                this.error =
                    Some("Could not reach ME Cloud. Check your connection and try again.".into());
            }
            if index % 11 == 3 {
                this.busy = true;
            }
            if index % 11 == 4 {
                this.account.complete = Some("alex@example.test".into());
            }
            if index % 11 >= 6 {
                provider(
                    this,
                    if index % 11 == 7 {
                        "provider-ready"
                    } else if index % 11 == 8 {
                        "provider-error"
                    } else if index % 11 == 9 {
                        "provider-working"
                    } else if index % 11 == 10 {
                        "provider-quota"
                    } else {
                        "provider"
                    },
                );
            }
            cx.notify();
        });
    }
    pub fn recovery_step(
        view: &Entity<MeApp>,
        request: &me_protocol::Registration,
        code: &str,
        cx: &mut App,
    ) {
        view.update(cx, |this, cx| {
            this.busy = false;
            this.error = None;
            this.account.mode = AccountMode::Register;
            this.account.complete = None;
            this.account.saved = false;
            this.account.pending = Some(Arc::new(account_ui::PendingAccount {
                root: this.root.clone(),
                server: "http://127.0.0.1:8787".into(),
                password: zeroize::Zeroizing::new("synthetic gallery password".into()),
                code: zeroize::Zeroizing::new(code.into()),
                operation: account_ui::PendingOperation::Register(crate::copy_registration(
                    request,
                )),
            }));
            cx.notify();
        });
    }
    fn provider(this: &mut MeApp, mode: &str) {
        this.stop_codex_setup();
        this.unlocked = true;
        this.settings.onboarding_complete = false;
        this.account.mode = AccountMode::Unlock;
        this.account.binding = Some(me_core::account::AccountBinding {
            server: "http://127.0.0.1:8787".into(),
            account: me_protocol::Account {
                id: "synthetic-preview".into(),
                email: "alex@example.test".into(),
                email_verified: false,
                revision: 1,
            },
        });
        this.codex_ready = mode == "provider-ready";
        this.codex_issue = Some(if mode == "provider-error" {
            me_agent::codex::SetupIssue::Connection(
                "Couldn’t connect to ChatGPT. Check your internet connection and try again.".into(),
            )
        } else if mode == "provider-quota" {
            me_agent::codex::SetupIssue::UsageLimit
        } else {
            me_agent::codex::SetupIssue::SignInRequired
        });
        this.codex_message = this.codex_issue.as_ref().unwrap().message();
        if mode == "provider-working" {
            this.codex_issue = None;
            this.codex_cancel = Some(Arc::new(std::sync::atomic::AtomicBool::new(false)));
            this.codex_message = "Sign in to ChatGPT in your browser to continue.".into();
        }
    }
    pub fn preview(
        view: &Entity<MeApp>,
        mode: String,
        request: me_protocol::Registration,
        code: zeroize::Zeroizing<String>,
        cx: &mut App,
    ) {
        view.update(cx, |_, cx| {
            cx.spawn(async move |this, cx| {
                for _ in 0..500 {
                    if this.update(cx, |this, _| !this.busy).unwrap() {
                        break;
                    }
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(10))
                        .await;
                }
                this.update(cx, |this, cx| {
                    this.account_email
                        .update(cx, |i, cx| i.set_text("alex@example.test", cx));
                    this.motion.reduced = std::env::args().any(|a| a == "reduced");
                    this.account.mode = match mode.as_str() {
                        "signin" | "signin-busy" => AccountMode::SignIn,
                        "recover" => AccountMode::Recover,
                        "unlock" => AccountMode::Unlock,
                        _ => AccountMode::Register,
                    };
                    if mode == "code" || mode == "code-busy" {
                        this.account.pending = Some(Arc::new(account_ui::PendingAccount {
                            root: this.root.clone(),
                            server: "http://127.0.0.1:8787".into(),
                            password: zeroize::Zeroizing::new("synthetic gallery password".into()),
                            code,
                            operation: account_ui::PendingOperation::Register(request),
                        }));
                    }
                    if mode.starts_with("provider") {
                        provider(this, &mode);
                    }
                    if mode.starts_with("unlock") {
                        provider(this, "provider");
                        this.unlocked = false;
                        this.initialized = true;
                        if mode == "unlock-error" {
                            this.error = Some("Wrong password, or this vault is damaged.".into());
                        }
                        if mode == "unlock-busy" {
                            this.busy = true;
                        }
                        if mode == "unlock-long" {
                            this.account.binding.as_mut().unwrap().account.email =
                                "alexandra.with.a.long.account.name@an-example-company.test".into();
                        }
                    }
                    if mode == "settings" {
                        provider(this, "provider-ready");
                        this.settings.onboarding_complete = true;
                        this.show_settings = true;
                    }
                    if mode == "busy" || mode.ends_with("-busy") {
                        this.busy = true;
                        this.password
                            .update(cx, |i, cx| i.set_text("synthetic gallery password", cx));
                        this.password_repeat
                            .update(cx, |i, cx| i.set_text("synthetic gallery password", cx));
                        this.account.saved = mode == "code-busy";
                    }
                    if mode == "error" {
                        this.error = Some(
                            "Could not reach ME Cloud. Check your connection and try again.".into(),
                        );
                    }
                    if mode == "complete" {
                        this.account.complete = Some("alex@example.test".into());
                    }
                    cx.notify();
                })
                .unwrap();
            })
            .detach();
        });
    }
}
use gpui::{
    App, Application, Bounds, KeyBinding, WindowBounds, WindowOptions, prelude::*, px, size,
};
gpui::actions!(
    account_gallery,
    [TogglePreviewSize, NextPreview, PreviewRecovery]
);
fn copy_registration(request: &me_protocol::Registration) -> me_protocol::Registration {
    me_protocol::Registration {
        email: request.email.clone(),
        auth_secret: request.auth_secret.clone(),
        recovery_secret: request.recovery_secret.clone(),
        vault: request.vault.clone(),
    }
}
fn main() {
    let root = std::path::PathBuf::from(
        std::env::var_os("ME_VAULT_DIR").expect("Set synthetic ME_VAULT_DIR"),
    );
    assert!(root.starts_with("/tmp") || root.starts_with("/private/tmp"));
    assert!(!root.exists());
    assert_eq!(std::env::var("ME_CODEX_BIN").unwrap(), "/usr/bin/false");
    let mode = std::env::args().nth(1).unwrap_or("create".into());
    let small = std::env::args().any(|a| a == "small");
    let code = me_core::account::generate_recovery_code().unwrap();
    let request = me_core::account::prepare_registration(
        &root,
        "alex@example.test",
        "synthetic gallery password",
        &code,
    )
    .unwrap();
    Application::new()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            assets::load_fonts(cx).unwrap();
            input::register_bindings(cx);
            cx.bind_keys([
                KeyBinding::new("enter", shell::Confirm, Some("Me")),
                KeyBinding::new("tab", shell::NextField, Some("Me")),
                KeyBinding::new("shift-tab", shell::PreviousField, Some("Me")),
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
                    |_, cx| cx.new(shell::MeApp::new),
                )
                .unwrap();
            window
                .update(cx, |_, window, _| {
                    window.resize(dimensions);
                    window.set_window_title("ME Account Preview — Synthetic");
                })
                .unwrap();
            let view = window.entity(cx).unwrap();
            let recovery_request = copy_registration(&request);
            let recovery_code = code.clone();
            shell::preview(&view, mode, request, code, cx);
            cx.bind_keys([
                KeyBinding::new("f6", TogglePreviewSize, None),
                KeyBinding::new("f7", NextPreview, None),
                KeyBinding::new("f8", PreviewRecovery, None),
            ]);
            cx.on_action(move |_: &PreviewRecovery, cx| {
                let request = copy_registration(&recovery_request);
                let code = recovery_code.clone();
                cx.defer(move |cx| {
                    if let Ok(view) = window.entity(cx) {
                        shell::recovery_step(&view, &request, &code, cx);
                    }
                });
            });
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
            let index = std::cell::Cell::new(0);
            cx.on_action(move |_: &NextPreview, cx| {
                let next = index.get();
                index.set(next + 1);
                cx.defer(move |cx| {
                    if let Ok(view) = window.entity(cx) {
                        shell::cycle(&view, next, cx);
                    }
                });
            });
            cx.activate(true);
        });
}
