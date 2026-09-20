//! Synthetic startup regression/visual harness. No real vault, account or network.
//! Set ME_VAULT_DIR=/tmp/<fresh-folder>/vault and ME_CODEX_BIN=/tmp/<fresh-folder>/fake-codex.
//! Modes: locked, create, quota, offline, workspace; verify-{ready,quota,offline} exits after assertions.
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

    pub fn preview(view: &Entity<MeApp>, mode: String, cx: &mut App) {
        view.update(cx, |_, cx| {
            cx.spawn(async move |this, cx| {
                let executor = cx.background_executor().clone();
                for _ in 0..500 {
                    if this.update(cx, |this, _| !this.busy).unwrap() {
                        break;
                    }
                    executor.timer(std::time::Duration::from_millis(10)).await;
                }
                this.update(cx, |this, cx| {
                    this.password.update(cx, |field, cx| {
                        field.set_text(
                            if mode == "wrong" {
                                "incorrect-password"
                            } else {
                                "synthetic-startup-password"
                            },
                            cx,
                        )
                    });
                    this.unlock_vault(cx);
                })
                .unwrap();
            })
            .detach();
        });
    }

    pub fn verify(view: &Entity<MeApp>, root: PathBuf, mode: String, cx: &mut App) {
        view.update(cx, |_, cx| {
            cx.spawn(async move |this, cx| {
                let executor = cx.background_executor().clone();
                for _ in 0..500 {
                    if this.update(cx, |this, _| !this.busy && this.codex_cancel.is_some()).unwrap() { break; }
                    executor.timer(std::time::Duration::from_millis(10)).await;
                }
                this.update(cx, |this, cx| {
                    assert!(this.initialized && !this.busy && !this.codex_ready);
                    this.password.update(cx, |field, cx| field.set_text("wrong-password", cx));
                    this.unlock_vault(cx);
                }).unwrap();
                for _ in 0..500 {
                    if this.update(cx, |this, _| !this.busy).unwrap() { break; }
                    executor.timer(std::time::Duration::from_millis(10)).await;
                }
                this.update(cx, |this, cx| {
                    assert!(!this.unlocked && this.error.is_some());
                    this.password.update(cx, |field, cx| field.set_text("synthetic-startup-password", cx));
                    this.unlock_vault(cx);
                }).unwrap();
                for _ in 0..500 {
                    if this.update(cx, |this, _| !this.busy).unwrap() { break; }
                    executor.timer(std::time::Duration::from_millis(10)).await;
                }
                this.update(cx, |this, cx| {
                    assert!(this.app_ready(), "Vault must open before Codex responds");
                    assert!(!this.ai_ready() && this.codex_cancel.is_some());
                    assert!(!this.show_codex_setup);
                    this.run_change(cx, |vault| {
                        vault.save_note(None, "Offline note", "Synthetic local value")?;
                        Ok("Saved locally".into())
                    });
                }).unwrap();
                for _ in 0..500 {
                    if this.update(cx, |this, _| !this.busy).unwrap() { break; }
                    executor.timer(std::time::Duration::from_millis(10)).await;
                }
                this.update(cx, |this, _| {
                    assert!(this.error.is_none());
                    assert!(this.collection.items.iter().any(|item| item.title == "Offline note"));
                }).unwrap();
                let release = root.parent().unwrap().join("release-check");
                executor.spawn(async move { std::fs::write(release, "ready").unwrap(); }).await;
                for _ in 0..500 {
                    if this.update(cx, |this, _| this.codex_cancel.is_none()).unwrap() { break; }
                    executor.timer(std::time::Duration::from_millis(10)).await;
                }
                this.update(cx, |this, cx| {
                    assert!(this.app_ready(), "Connection result must never relock the vault");
                    assert!(this.codex_cancel.is_none());
                    assert!(!this.show_codex_setup, "Passive checks must not take over the workspace");
                    if mode == "verify-ready" {
                        assert!(this.ai_ready());
                        assert!(!this.codex_notice && this.codex_issue.is_none());
                    } else {
                        assert!(!this.ai_ready() && this.codex_notice && this.codex_issue.is_some());
                    }
                    // A failed provider must still permit a subsequent local write.
                    this.run_change(cx, |vault| {
                        vault.save_note(None, "After check", "Still local")?;
                        Ok("Saved locally".into())
                    });
                }).unwrap();
                for _ in 0..500 {
                    if this.update(cx, |this, _| !this.busy).unwrap() { break; }
                    executor.timer(std::time::Duration::from_millis(10)).await;
                }
                this.update(cx, |this, _| {
                    assert!(this.error.is_none());
                    assert!(this.collection.items.iter().any(|item| item.title == "After check"));
                }).unwrap();
                let audit = executor.spawn(async move { std::fs::read_to_string(root.parent().unwrap().join("calls")).unwrap() }).await;
                assert_eq!(audit.lines().filter(|line| *line == "initialize").count(), 1, "Unlock must not repeat the startup check");
                assert!(!audit.contains("account/login/start"));
                assert!(!audit.contains("turn/"));
                println!("PASS {mode}: wrong password stays locked; correct password and local writes work during and after the check; one passive connection check only.");
                cx.update(|cx| cx.quit()).unwrap();
            }).detach();
        });
    }
}
use gpui::{
    App, Application, Bounds, KeyBinding, WindowBounds, WindowOptions, prelude::*, px, size,
};
use std::os::unix::fs::PermissionsExt;
fn main() {
    let root = std::path::PathBuf::from(
        std::env::var_os("ME_VAULT_DIR").expect("Set a fresh synthetic ME_VAULT_DIR"),
    );
    assert!(root.starts_with("/tmp") || root.starts_with("/private/tmp"));
    let parent = root.parent().unwrap();
    let binary = parent.join("fake-codex");
    assert_eq!(
        std::path::PathBuf::from(
            std::env::var_os("ME_CODEX_BIN").expect("Set the synthetic ME_CODEX_BIN")
        ),
        binary
    );
    let mode = std::env::args().nth(1).unwrap_or_else(|| "locked".into());
    let small = std::env::args().any(|arg| arg == "small");
    std::fs::create_dir_all(parent).unwrap();
    if mode != "create" {
        let mut vault = me_core::Vault::create(&root, "synthetic-startup-password").unwrap();
        vault.set_automatic_evaluation(false).unwrap();
        vault
            .save_note(None, "A note for you", "This is synthetic preview data.")
            .unwrap();
    }
    let script = r#"#!/usr/bin/env python3
import json, pathlib, sys, time
root=pathlib.Path(sys.argv[0]).parent
mode='MODE'
for line in sys.stdin:
    request=json.loads(line)
    method=request.get('method')
    with (root/'calls').open('a') as audit: audit.write(method+'\n')
    if 'id' not in request: continue
    if method=='initialize':
        while not (root/'release-check').exists(): time.sleep(.01)
        result={}
    elif method=='account/read': result={'account':{'type':'chatgpt'}}
    elif method=='model/list': result={'data':[{'model':'gpt-5.6-sol'}],'nextCursor':None}
    elif method=='account/rateLimits/read':
        if 'offline' in mode:
            print(json.dumps({'id':request['id'],'error':{'code':-1,'message':'Synthetic connection failure'}}),flush=True)
            continue
        result={'rateLimits':{'primary':{'usedPercent':100 if 'quota' in mode else 20}}}
    else: raise AssertionError('Unexpected provider operation: '+method)
    print(json.dumps({'id':request['id'],'result':result}),flush=True)
"#.replace("MODE", &mode);
    std::fs::write(&binary, script).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    if !mode.starts_with("verify-") && mode != "locked" {
        std::fs::write(parent.join("release-check"), "ready").unwrap();
    }
    Application::new()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            assets::load_fonts(cx).unwrap();
            input::register_bindings(cx);
            cx.bind_keys([
                KeyBinding::new("enter", shell::Confirm, Some("Me")),
                KeyBinding::new("escape", shell::Dismiss, Some("Me")),
                KeyBinding::new("tab", shell::NextField, Some("Me")),
                KeyBinding::new("shift-tab", shell::PreviousField, Some("Me")),
                KeyBinding::new("cmd-shift-l", shell::LockVault, Some("Me")),
                KeyBinding::new("cmd-,", shell::OpenSettings, Some("Me")),
            ]);
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                            None,
                            size(
                                px(if small { 800. } else { 1120. }),
                                px(if small { 600. } else { 820. }),
                            ),
                            cx,
                        ))),
                        ..Default::default()
                    },
                    |_, cx| cx.new(shell::MeApp::new),
                )
                .unwrap();
            window
                .update(cx, |_, window, _| {
                    window.set_window_title("ME Startup — Synthetic");
                    window.resize(size(
                        px(if small { 800. } else { 1120. }),
                        px(if small { 600. } else { 820. }),
                    ));
                })
                .unwrap();
            if mode.starts_with("verify-") {
                shell::verify(&window.entity(cx).unwrap(), root.clone(), mode.clone(), cx);
            } else if matches!(mode.as_str(), "workspace" | "offline" | "quota" | "wrong") {
                shell::preview(&window.entity(cx).unwrap(), mode.clone(), cx);
            }
            cx.activate(true);
        });
}
