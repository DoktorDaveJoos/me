//! Native behavioral harness, isolated to a fresh temporary vault and fake provider.
//! ME_VAULT_DIR=/tmp/<fresh>/vault ME_CODEX_BIN=/tmp/<fresh>/fake-codex cargo run -p me-app --example onboarding_flow
//! Optionally set ME_ACCOUNT_API_URL to a disposable loopback API to register through the real HTTP client.
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
    pub fn verify(view: &Entity<MeApp>, root: PathBuf, register: bool, cx: &mut App) {
        view.update(cx, |_, cx| {
            cx.spawn(async move |this, cx| {
                let executor = cx.background_executor().clone();
                macro_rules! wait_for {
                    ($condition:expr) => {{
                        let mut matched = false;
                        for _ in 0..1000 {
                            if this.update(cx, |app, _| $condition(app)).unwrap() { matched = true; break; }
                            executor.timer(std::time::Duration::from_millis(10)).await;
                        }
                        assert!(matched, "Timed out: {}", stringify!($condition));
                    }};
                }
                wait_for!(|app: &MeApp| !app.busy && app.codex_cancel.is_none() && app.motion.loaded);
                let entrance_started = this.update(cx, |app, _| app.motion.window_started).unwrap();
                let mut original_recovery = None;
                if register {
                    this.update(cx, |app, cx| {
                        let email = format!("onboarding-{}@example.test", uuid::Uuid::new_v4());
                        app.account_email.update(cx, |i, cx| i.set_text(&email, cx));
                        app.password.update(cx, |i, cx| i.set_text("synthetic-onboarding-password", cx));
                        app.password_repeat.update(cx, |i, cx| i.set_text("synthetic-onboarding-password", cx));
                        app.submit_account(cx);
                        assert!(app.busy);
                        assert_eq!(app.password.read(cx).content.as_ref(), "synthetic-onboarding-password", "Submitted password stays visible while preparing");
                        assert_eq!(app.password_repeat.read(cx).content.as_ref(), "synthetic-onboarding-password");
                    }).unwrap();
                    wait_for!(|app: &MeApp| !app.busy);
                    this.update(cx, |app, cx| {
                        assert!(app.account.pending.is_some() && app.error.is_none());
                        assert_eq!(app.motion.window_started, entrance_started, "Recovery step must keep the window entrance clock");
                        assert!(app.password.read(cx).content.is_empty() && app.password_repeat.read(cx).content.is_empty(), "Successful transition clears hidden fields");
                        app.submit_account(cx);
                        assert!(!app.busy && app.error.is_some(), "Safekeeping must be acknowledged");
                        original_recovery = Some(app.account.pending.as_ref().unwrap().code.clone());
                        app.account.saved = true;
                        app.submit_account(cx);
                    }).unwrap();
                } else {
                    this.update(cx, |app, cx| {
                        app.password.update(cx, |i, cx| i.set_text("synthetic-onboarding-password", cx));
                        app.unlock_vault(cx);
                    }).unwrap();
                }
                wait_for!(|app: &MeApp| !app.busy);
                this.update(cx, |app, cx| {
                    assert!(app.unlocked && app.provider_setup_visible() && !app.app_ready());
                    assert_eq!(app.motion.window_started, entrance_started, "Provider step must keep the window entrance clock");
                    assert!(!app.ai_ready() && app.error.is_none());
                    assert!(app.account.pending.is_none());
                    app.finish_onboarding(cx);
                    assert!(!app.busy && !app.settings.onboarding_complete);
                }).unwrap();
                // Drop and reopen the real encrypted session before provider setup.
                for completed in [false, true] {
                    let old = this.update(cx, |app, cx| app.detach_vault(cx)).unwrap();
                    executor.spawn(async move { old.lock().unwrap().take(); }).await;
                    this.update(cx, |app, cx| {
                        app.busy = false;
                        app.settings = me_core::AppSettings::default();
                        app.codex_ready = false;
                        app.password.update(cx, |i, cx| i.set_text("synthetic-onboarding-password", cx));
                        app.unlock_vault(cx);
                    }).unwrap();
                    wait_for!(|app: &MeApp| !app.busy);
                    this.update(cx, |app, _| {
                        assert_eq!(app.settings.onboarding_complete, completed);
                        assert_eq!(app.provider_setup_visible(), !completed);
                        assert_eq!(app.app_ready(), completed);
                        assert!(!app.ai_ready());
                    }).unwrap();
                    if completed { break; }
                    for phase in ["wait", "offline", "quota", "ready"] {
                        let file = root.parent().unwrap().join("phase");
                        executor.spawn(async move { std::fs::write(file, phase).unwrap(); }).await;
                        this.update(cx, |app, cx| app.start_codex_setup(false, cx)).unwrap();
                        if phase == "wait" {
                            executor.timer(std::time::Duration::from_millis(200)).await;
                            this.update(cx, |app, cx| { app.stop_codex_setup(); cx.notify(); }).unwrap();
                        }
                        wait_for!(|app: &MeApp| app.codex_cancel.is_none());
                        this.update(cx, |app, cx| {
                            assert!(app.provider_setup_visible() && !app.app_ready());
                            assert_eq!(app.codex_ready, phase == "ready");
                            app.finish_onboarding(cx);
                            assert_eq!(app.busy, phase == "ready");
                        }).unwrap();
                        wait_for!(|app: &MeApp| !app.busy);
                    }
                    this.update(cx, |app, _| { assert!(app.app_ready() && app.ai_ready()); }).unwrap();
                }
                if register {
                    let first_email = this.update(cx, |app, _| app.account.binding.as_ref().unwrap().account.email.clone()).unwrap();
                    this.update(cx, |app, cx| app.run_change(cx, |vault| {
                        vault.save_note(None, "First account only", "Synthetic first account content")?;
                        Ok("Saved".into())
                    })).unwrap();
                    wait_for!(|app: &MeApp| !app.busy);
                    this.update(cx, |app, cx| app.log_out(cx)).unwrap();
                    wait_for!(|app: &MeApp| !app.busy);
                    this.update(cx, |app, _| {
                        assert!(!app.unlocked && app.account.binding.is_none() && app.account.signed_out);
                        assert!(app.account.mode == AccountMode::SignIn);
                        assert!(!app.codex_ready && app.codex_cancel.is_none());
                    }).unwrap();
                    let base = root.clone();
                    let signed_out = executor.spawn(async move { device_accounts::load(&base).unwrap().signed_out }).await;
                    assert!(signed_out, "Logout survives a restart");
                    this.update(cx, |app, cx| app.set_account_mode(AccountMode::Register, cx)).unwrap();
                    wait_for!(|app: &MeApp| !app.busy);
                    let second_email = format!("second-{}@example.test", uuid::Uuid::new_v4());
                    this.update(cx, |app, cx| {
                        assert!(app.root.as_ref().unwrap() != &root);
                        app.account_email.update(cx, |i, cx| i.set_text(&second_email, cx));
                        app.password.update(cx, |i, cx| i.set_text("synthetic-second-password", cx));
                        app.password_repeat.update(cx, |i, cx| i.set_text("synthetic-second-password", cx));
                        app.submit_account(cx);
                    }).unwrap();
                    wait_for!(|app: &MeApp| !app.busy);
                    this.update(cx, |app, cx| {
                        assert!(app.account.pending.is_some() && app.error.is_none());
                        app.account.saved = true; app.submit_account(cx);
                    }).unwrap();
                    wait_for!(|app: &MeApp| !app.busy && app.codex_cancel.is_none());
                    let second_root = this.update(cx, |app, cx| {
                        assert!(app.provider_setup_visible() && app.codex_ready && app.error.is_none());
                        assert!(app.collection.items.is_empty());
                        app.finish_onboarding(cx); app.root.clone().unwrap()
                    }).unwrap();
                    wait_for!(|app: &MeApp| !app.busy);
                    this.update(cx, |app, cx| app.run_change(cx, |vault| {
                        vault.save_note(None, "Second account only", "Synthetic second account content")?;
                        Ok("Saved".into())
                    })).unwrap();
                    wait_for!(|app: &MeApp| !app.busy);
                    // Re-run startup selection from disk after closing the second vault.
                    let old = this.update(cx, |app, cx| { app.stop_codex_setup(); app.detach_vault(cx) }).unwrap();
                    executor.spawn(async move { old.lock().unwrap().take(); }).await;
                    this.update(cx, |app, cx| {
                        app.root = Some(root.clone()); app.account = AccountState::default(); app.initialized = false;
                        app.account_email.update(cx, |i, cx| i.set_text("", cx));
                        MeApp::inspect_vault(cx);
                    }).unwrap();
                    wait_for!(|app: &MeApp| !app.busy && app.codex_cancel.is_none());
                    this.update(cx, |app, cx| {
                        assert_eq!(app.root.as_ref().unwrap(), &second_root);
                        assert_eq!(app.account.binding.as_ref().unwrap().account.email, second_email);
                        assert!(app.account.mode == AccountMode::Unlock && !app.unlocked);
                        app.password.update(cx, |i, cx| i.set_text("synthetic-onboarding-password", cx));
                        app.unlock_vault(cx);
                    }).unwrap();
                    wait_for!(|app: &MeApp| !app.busy);
                    this.update(cx, |app, cx| {
                        assert!(!app.unlocked && app.error.is_some(), "First account password must not unlock the second vault");
                        assert_eq!(app.password.read(cx).content.as_ref(), "synthetic-onboarding-password", "Failure preserves the password for correction");
                        app.password.update(cx, |i, cx| i.set_text("synthetic-second-password", cx));
                        app.unlock_vault(cx);
                    }).unwrap();
                    wait_for!(|app: &MeApp| !app.busy);
                    this.update(cx, |app, cx| {
                        assert!(app.app_ready() && app.error.is_none());
                        assert!(app.password.read(cx).content.is_empty(), "Unlock clears the completed password");
                        assert_eq!(app.collection.items.len(), 1);
                        assert_eq!(app.collection.items[0].title, "Second account only");
                        app.log_out(cx);
                    }).unwrap();
                    wait_for!(|app: &MeApp| !app.busy);
                    this.update(cx, |app, cx| {
                        app.account_email.update(cx, |i, cx| i.set_text(&first_email, cx));
                        app.password.update(cx, |i, cx| i.set_text("synthetic-onboarding-password", cx));
                        app.submit_account(cx);
                    }).unwrap();
                    wait_for!(|app: &MeApp| !app.busy);
                    this.update(cx, |app, _| {
                        assert!(app.app_ready() && app.error.is_none());
                        assert_eq!(app.root.as_ref().unwrap(), &root);
                        assert_eq!(app.collection.items.len(), 1);
                        assert_eq!(app.collection.items[0].title, "First account only");
                    }).unwrap();
                    // Recovery after logout must locate and update this same original vault.
                    this.update(cx, |app, cx| app.log_out(cx)).unwrap();
                    wait_for!(|app: &MeApp| !app.busy);
                    this.update(cx, |app, cx| {
                        app.set_account_mode(AccountMode::Recover, cx);
                        app.account_email.update(cx, |i, cx| i.set_text(&first_email, cx));
                        app.recovery_input.update(cx, |i, cx| i.set_text(original_recovery.as_ref().unwrap(), cx));
                        app.password.update(cx, |i, cx| i.set_text("synthetic-recovered-password", cx));
                        app.password_repeat.update(cx, |i, cx| i.set_text("synthetic-recovered-password", cx));
                        app.submit_account(cx);
                    }).unwrap();
                    wait_for!(|app: &MeApp| !app.busy);
                    this.update(cx, |app, cx| {
                        assert!(app.account.pending.is_some() && app.error.is_none());
                        assert_eq!(app.account.pending.as_ref().unwrap().root.as_ref().unwrap(), &root);
                        app.account.saved = true; app.submit_account(cx);
                    }).unwrap();
                    wait_for!(|app: &MeApp| !app.busy);
                    this.update(cx, |app, _| {
                        assert!(app.app_ready() && app.error.is_none());
                        assert_eq!(app.root.as_ref().unwrap(), &root);
                        assert_eq!(app.collection.items[0].title, "First account only");
                    }).unwrap();
                    println!("PASS account lifecycle: logout persists; new registration uses a separate vault; startup remembers the latest account; only its password unlocks it; sign-in and recovery find the original vault and preserve its data.");
                }
                let file = root.parent().unwrap().join("calls");
                let calls = executor.spawn(async move { std::fs::read_to_string(file).unwrap() }).await;
                assert!(!calls.contains("turn/") && !calls.contains("thread/start"));
                println!("PASS: registration/unlock -> provider gate; interrupted setup resumes; cancel/offline/quota cannot complete; successful check and explicit finish persist; later unlock works offline; no model turns.");
                cx.update(|cx| cx.quit()).unwrap();
            }).detach();
        });
    }
}
use gpui::{App, Application, Bounds, WindowBounds, WindowOptions, prelude::*, px, size};
use std::os::unix::fs::PermissionsExt;
fn main() {
    let root = std::path::PathBuf::from(
        std::env::var_os("ME_VAULT_DIR").expect("Set a fresh temporary vault"),
    );
    assert!(root.starts_with("/tmp") || root.starts_with("/private/tmp"));
    assert!(!root.exists());
    let parent = root.parent().unwrap();
    let binary = parent.join("fake-codex");
    assert_eq!(
        std::path::PathBuf::from(std::env::var_os("ME_CODEX_BIN").unwrap()),
        binary
    );
    std::fs::create_dir_all(parent).unwrap();
    let register = std::env::var("ME_ACCOUNT_API_URL").is_ok_and(|s| {
        assert!(
            s.starts_with("http://127.0.0.1:"),
            "Use a disposable loopback API"
        );
        true
    });
    if !register {
        let code = me_core::account::generate_recovery_code().unwrap();
        let request = me_core::account::prepare_registration(
            &root,
            "synthetic@example.test",
            "synthetic-onboarding-password",
            &code,
        )
        .unwrap();
        me_core::account::stage_registration(&root, "synthetic-onboarding-password", &request)
            .unwrap();
        let response = me_protocol::AccountResponse {
            account: me_protocol::Account {
                id: uuid::Uuid::new_v4().to_string(),
                email: request.email.clone(),
                email_verified: false,
                revision: 1,
            },
            vault: request.vault.clone(),
        };
        let mut vault = me_core::account::open_account(
            &root,
            "synthetic-onboarding-password",
            "http://127.0.0.1:8787",
            &response,
            false,
        )
        .unwrap();
        vault.set_automatic_evaluation(false).unwrap();
    }
    std::fs::write(parent.join("phase"), "missing").unwrap();
    std::fs::write(&binary, r#"#!/usr/bin/env python3
import json,pathlib,sys,time
root=pathlib.Path(sys.argv[0]).parent
phase=(root/'phase').read_text()
for line in sys.stdin:
 r=json.loads(line); m=r.get('method')
 with (root/'calls').open('a') as f: f.write(m+'\n')
 if 'id' not in r: continue
 if m=='initialize':
  if phase=='wait': time.sleep(30)
  result={}
 elif m=='account/read': result={'account':None if phase=='missing' else {'type':'chatgpt'}}
 elif m=='model/list': result={'data':[{'model':'gpt-5.6-sol'}],'nextCursor':None}
 elif m=='account/rateLimits/read':
  if phase=='offline':
   print(json.dumps({'id':r['id'],'error':{'code':-1,'message':'Synthetic offline failure'}}),flush=True);continue
  result={'rateLimits':{'primary':{'usedPercent':100 if phase=='quota' else 20}}}
 else: raise AssertionError('Unexpected provider call: '+m)
 print(json.dumps({'id':r['id'],'result':result}),flush=True)
"#).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    Application::new()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            assets::load_fonts(cx).unwrap();
            input::register_bindings(cx);
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                            None,
                            size(px(1120.), px(820.)),
                            cx,
                        ))),
                        ..Default::default()
                    },
                    |_, cx| cx.new(shell::MeApp::new),
                )
                .unwrap();
            shell::verify(&window.entity(cx).unwrap(), root.clone(), register, cx);
        });
}
