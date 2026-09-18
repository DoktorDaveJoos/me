use super::*;
use std::os::unix::fs::PermissionsExt;

fn fake(path: &Path, mode: &str) {
    let script = r##"#!/usr/bin/env python3
import sys,json,os,pathlib
mode='MODE'
home=pathlib.Path(os.environ['CODEX_HOME'])
signed=(home/'synthetic-login').exists() or mode not in ['missing','login','failed','url','cancel','unrelated']
audit=pathlib.Path(sys.argv[0]+'.calls')
if mode=='wrapper':
 import subprocess
 child=subprocess.Popen([sys.executable,'-c','import time; time.sleep(30)'],stdin=subprocess.DEVNULL)
 (home/'synthetic-child-pid').write_text(str(child.pid))
def send(x): print(json.dumps(x),flush=True)
for line in sys.stdin:
 r=json.loads(line);m=r.get('method');i=r.get('id');p=r.get('params',{})
 with audit.open('a') as f: f.write(m+'\n')
 if i is None: continue
 if m=='initialize': result={}
 elif m=='account/read':
  assert p['refreshToken'] is True
  if mode=='expired':
   send({'id':i,'error':{'code':-1,'message':'synthetic refresh failure'}});continue
  result={'account':{'type':'apiKey' if mode=='apikey' else 'chatgpt'} if signed else None}
 elif m=='account/login/start':
  assert p=={'type':'chatgpt'}
  # Unrelated and early notifications must not complete this login.
  send({'method':'account/login/completed','params':{'loginId':'old-login','success':False}})
  result={'type':'chatgpt','loginId':'new-login','authUrl':'https://example.invalid/' if mode=='url' else 'https://auth.openai.com/synthetic'}
  send({'id':i,'result':result})
  if mode in ['cancel','unrelated']: continue
  signed=mode!='failed'
  if signed: (home/'synthetic-login').write_text('test fixture, no credentials')
  send({'method':'account/login/completed','params':{'loginId':'new-login','success':signed}})
  continue
 elif m=='model/list':
  if mode=='model': result={'data':[],'nextCursor':None}
  elif not p.get('cursor'): result={'data':[{'model':'another-model'}],'nextCursor':'second'}
  else: result={'data':[{'model':'gpt-5.6-sol'}],'nextCursor':None}
 elif m=='account/rateLimits/read':
  if mode=='offline':
   send({'id':i,'error':{'code':-1,'message':'synthetic unavailable'}});continue
  # Exhausted quota is a configured account, not a reason to hide stored data.
  result={'rateLimits':{'primary':{'usedPercent':100}}}
 else: raise AssertionError('Setup must not start model or tool work: '+m)
 send({'id':i,'result':result})
"##.replace("MODE", mode);
    fs::write(path, script).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
fn calls(bin: &Path) -> String {
    fs::read_to_string(format!("{}.calls", bin.display())).unwrap_or_default()
}

#[test]
fn setup_check_requires_chatgpt_model_and_a_live_authenticated_connection() {
    for (mode, expected) in [
        ("missing", Some(SetupIssue::SignInRequired)),
        ("apikey", Some(SetupIssue::WrongAccount)),
        ("model", Some(SetupIssue::ModelUnavailable)),
        ("ready", None),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("codex");
        fake(&bin, mode);
        let mut progress = Vec::new();
        let result = setup_with_binary(
            &temp.path().join("home"),
            false,
            Arc::new(AtomicBool::new(false)),
            &mut |e| progress.push(e),
            &bin,
        );
        match expected {
            Some(issue) => assert_eq!(result.unwrap_err(), issue),
            None => result.unwrap(),
        };
        assert!(progress.is_empty());
        assert!(!calls(&bin).contains("account/login/start"));
        assert!(!calls(&bin).contains("thread/"));
    }
    for mode in ["expired", "offline"] {
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("codex");
        fake(&bin, mode);
        assert!(matches!(
            setup_with_binary(
                &temp.path().join("home"),
                false,
                Arc::new(AtomicBool::new(false)),
                &mut |_| {},
                &bin
            ),
            Err(SetupIssue::Connection(_))
        ));
    }
}

#[test]
fn browser_login_is_checked_and_persisted_login_can_be_used_on_restart() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("codex");
    fake(&bin, "login");
    let home = temp.path().join("home");
    let mut urls = Vec::new();
    setup_with_binary(
        &home,
        true,
        Arc::new(AtomicBool::new(false)),
        &mut |e| {
            if let Progress::LoginUrl(url) = e {
                urls.push(url)
            }
        },
        &bin,
    )
    .unwrap();
    assert_eq!(urls, vec!["https://auth.openai.com/synthetic"]);
    setup_with_binary(
        &home,
        false,
        Arc::new(AtomicBool::new(false)),
        &mut |_| panic!("Restart must not prompt for login"),
        &bin,
    )
    .unwrap();
    assert_eq!(calls(&bin).matches("account/login/start").count(), 1);
    assert_eq!(calls(&bin).matches("account/rateLimits/read").count(), 2);
    assert!(!calls(&bin).contains("thread/"));
    assert!(!calls(&bin).contains("turn/"));
    // A removed sign-in must take the next check back to setup.
    fs::remove_file(home.join("synthetic-login")).unwrap();
    assert_eq!(
        setup_with_binary(
            &home,
            false,
            Arc::new(AtomicBool::new(false)),
            &mut |_| {},
            &bin
        )
        .unwrap_err(),
        SetupIssue::SignInRequired
    );
}

#[test]
fn failed_unrelated_cancelled_or_untrusted_logins_never_unlock_the_gate() {
    for mode in ["failed", "url", "cancel", "unrelated"] {
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("codex");
        fake(&bin, mode);
        let cancel = Arc::new(AtomicBool::new(false));
        let trigger = cancel.clone();
        let start = Instant::now();
        let mut opened = false;
        let result = setup_with_binary(
            &temp.path().join("home"),
            true,
            cancel,
            &mut |e| {
                if let Progress::LoginUrl(_) = e {
                    opened = true;
                    if mode == "cancel" || mode == "unrelated" {
                        trigger.store(true, Ordering::SeqCst);
                    }
                }
            },
            &bin,
        );
        assert!(result.is_err(), "{mode}");
        if mode == "url" {
            assert!(!opened);
        }
        assert!(!calls(&bin).contains("account/rateLimits/read"));
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}

#[test]
fn missing_cli_and_foreign_configuration_are_actionable_setup_failures() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    assert!(
        setup_with_binary(
            &home,
            false,
            Arc::new(AtomicBool::new(false)),
            &mut |_| {},
            &temp.path().join("missing")
        )
        .unwrap_err()
        .message()
        .contains("Codex CLI")
    );
    let bin = temp.path().join("codex");
    fake(&bin, "ready");
    fs::write(
        home.join("config.toml"),
        "# synthetic foreign configuration",
    )
    .unwrap();
    assert!(
        setup_with_binary(
            &home,
            false,
            Arc::new(AtomicBool::new(false)),
            &mut |_| {},
            &bin
        )
        .is_err()
    );
    assert!(calls(&bin).is_empty());
}

#[test]
fn extraction_requests_setup_instead_of_starting_an_embedded_login() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("codex");
    fake(&bin, "missing");
    let input = ExtractionInput {
        run_id: "synthetic".into(),
        source_id: "synthetic".into(),
        title: "Synthetic".into(),
        segments: vec![me_core::SourceText {
            segment_id: "synthetic".into(),
            ordinal: 1,
            text: "Synthetic source".into(),
        }],
    };
    let mut needs_setup = false;
    assert!(
        extract_with_binary(
            &temp.path().join("home"),
            &input,
            Arc::new(AtomicBool::new(false)),
            &mut |e| {
                match e {
                    Progress::SetupRequired(SetupIssue::SignInRequired) => needs_setup = true,
                    Progress::LoginUrl(_) => panic!("Inbox must redirect to setup"),
                    _ => {}
                }
            },
            &bin
        )
        .is_err()
    );
    assert!(needs_setup);
    assert!(!calls(&bin).contains("account/login/start"));
    assert!(!calls(&bin).contains("thread/"));
}

#[test]
#[ignore = "Needs the real Codex CLI; temporary home, no browser, no model turn"]
fn installed_cli_starts_and_offers_login_with_a_desktop_path() {
    let temp = tempfile::tempdir().unwrap();
    let start = Instant::now();
    let mut command =
        Session::command(&temp.path().join("home"), temp.path(), &executable()).unwrap();
    // Finder/Dock launches do not inherit Homebrew's Node from the shell PATH.
    // Override this command only; tests must not change the process environment.
    command.env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin");
    let mut session = Session::spawn(command, Arc::new(AtomicBool::new(false))).unwrap();
    session.deadline = Instant::now() + Duration::from_secs(30);
    initialize(&mut session).unwrap();
    assert_eq!(
        verify_configuration(&mut session).unwrap_err(),
        SetupIssue::SignInRequired
    );
    let login = session
        .rpc("account/login/start", json!({"type":"chatgpt"}))
        .unwrap();
    // Do not emit the OAuth URL, login ID, or any credentials, even on failure.
    assert!(
        login
            .get("authUrl")
            .and_then(Value::as_str)
            .is_some_and(|url| url.starts_with("https://auth.openai.com/")
                || url.starts_with("https://chatgpt.com/"))
    );
    let login_id = login
        .get("loginId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .expect("Login ID is required");
    session
        .rpc("account/login/cancel", json!({"loginId":login_id}))
        .unwrap();
    drop(session);
    assert!(start.elapsed() < Duration::from_secs(40));
}

#[test]
fn setup_teardown_stops_launcher_children_and_closes_inherited_pipes() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("codex");
    fake(&bin, "wrapper");
    let start = Instant::now();
    setup_with_binary(
        &temp.path().join("home"),
        false,
        Arc::new(AtomicBool::new(false)),
        &mut |_| {},
        &bin,
    )
    .unwrap();
    assert!(start.elapsed() < Duration::from_secs(5));
}

#[test]
fn codex_generated_plugin_cache_does_not_invalidate_the_connection() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let bin = temp.path().join("fake-codex");
    fake(&bin, "ready");
    // Command creates the private home before we simulate Codex's own cache.
    let _ = Session::command(&home, temp.path(), &bin).unwrap();
    fs::create_dir_all(home.join("plugins/cache/synthetic")).unwrap();
    fs::write(home.join("plugins/cache/synthetic/plugin.json"), "{}").unwrap();
    setup_with_binary(
        &home,
        false,
        Arc::new(AtomicBool::new(false)),
        &mut |_| {},
        &bin,
    )
    .unwrap();
}
