//! Bounded, text-only Codex App Server client. Auth is managed by Codex itself.
use me_core::{ExtractionInput, ExtractionOutput};
use me_diagnostics::{Field as F, record};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    fs,
    io::{BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

#[path = "codex_pipeline.rs"]
mod pipeline;
use pipeline::Retry;
pub use pipeline::{Checkpoints, ExtractionReport, NoCheckpoints, PIPELINE};

#[path = "assistant.rs"]
mod assistant;
pub use assistant::{
    ChatAnswer, answer_question, filter_fields, inspect_form, open_form_workspace,
    organize_documents,
};

pub const INBOX_MODEL: &str = "gpt-5.6-sol";
const INBOX_EFFORT: &str = "medium";
const MAX_BATCH_FACTS: usize = 96;
#[path = "codex_document.rs"]
mod document;
use document::extract_batch;
pub enum Progress {
    Stage(me_core::ImportStage),
    Units { current: u32, total: u32 },
    Message(String),
    LoginUrl(String),
    SetupRequired(SetupIssue),
}
pub type Result<T> = std::result::Result<T, String>;
const FAILURE: &str =
    "Check your ChatGPT connection and usage limit, then try again. Your files are saved.";
fn failed(_: impl std::fmt::Debug) -> String {
    FAILURE.into()
}
#[path = "codex_launcher.rs"]
mod launcher;
fn executable() -> PathBuf {
    launcher::find()
}

struct Session {
    child: Child,
    input: ChildStdin,
    events: mpsc::Receiver<Value>,
    pending: VecDeque<Value>,
    reader: Option<thread::JoinHandle<()>>,
    cancel: Arc<AtomicBool>,
    deadline: Instant,
    next: u64,
    retry: Retry,
}
impl Drop for Session {
    fn drop(&mut self) {
        // The npm launcher starts a native child. Kill only our dedicated process
        // group so teardown also closes inherited pipes and browser callbacks.
        if self.child.id() > 1 {
            let _ = rustix::process::kill_process_group(
                rustix::process::Pid::from_child(&self.child),
                rustix::process::Signal::KILL,
            );
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
    }
}
impl Session {
    fn start(home: &Path, scratch: &Path, cancel: Arc<AtomicBool>, binary: &Path) -> Result<Self> {
        Self::spawn(Self::command(home, scratch, binary)?, cancel)
    }
    fn command(home: &Path, scratch: &Path, binary: &Path) -> Result<Command> {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        if !home.exists() {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(home)
                .map_err(failed)?;
        }
        let meta = fs::symlink_metadata(home).map_err(failed)?;
        if !meta.is_dir() || meta.file_type().is_symlink() || meta.permissions().mode() & 0o077 != 0
        {
            return Err("The Codex configuration folder must be private (0700).".into());
        }
        // This home is owned by ME; refuse foreign configuration instead of
        // merging in MCP servers, hooks or alternate model providers. Codex can
        // create a plugin cache itself; disable plugins instead of rejecting it.
        if home.join("config.toml").exists() || home.join("AGENTS.md").exists() {
            return Err("The connection contains unsupported customizations. Use the dedicated ME. configuration.".into());
        }
        use std::os::unix::process::CommandExt;
        let mut cmd = Command::new(launcher::resolve(binary)?);
        cmd.process_group(0);
        cmd.env("CODEX_HOME", home)
            .env_remove("OPENAI_API_KEY")
            .env_remove("CODEX_API_KEY")
            .current_dir(scratch)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for setting in [
            "model_provider=\"openai\"",
            "default_permissions=\"me-inbox\"",
            "permissions.me-inbox.network.enabled=false",
            "features.shell_tool=false",
            "features.unified_exec=false",
            "features.apply_patch_freeform=false",
            "features.apps=false",
            "features.plugins=false",
            "features.remote_plugin=false",
            "features.skill_search=false",
            "features.skill_mcp_dependency_install=false",
            "features.computer_use=false",
            "features.browser_use=false",
            "features.browser_use_external=false",
            "features.image_generation=false",
            "features.code_mode_host=false",
            "features.hooks=false",
            "features.multi_agent=false",
            "project_doc_max_bytes=0",
            "web_search=\"disabled\"",
            "history.persistence=\"none\"",
            "cli_auth_credentials_store=\"file\"",
        ] {
            cmd.arg("-c").arg(setting);
        }
        cmd.arg("-c").arg(format!(
            "log_dir={}",
            json!(scratch.join("logs").to_string_lossy())
        ));
        cmd.arg("-c").arg(format!(
            "sqlite_home={}",
            json!(scratch.join("state").to_string_lossy())
        ));
        cmd.arg("-c").arg(format!(
            "permissions.me-inbox.filesystem={{\":root\"=\"deny\",\":minimal\"=\"read\",{}=\"read\"}}",
            json!(scratch.to_string_lossy())
        ));
        cmd.args(["app-server", "--listen", "stdio://"]);
        Ok(cmd)
    }
    fn spawn(mut cmd: Command, cancel: Arc<AtomicBool>) -> Result<Self> {
        let mut child = cmd
            .spawn()
            .map_err(|_| "Codex CLI couldn't be found or started.".to_string())?;
        let input = child.stdin.take().ok_or(FAILURE)?;
        let stdout = child.stdout.take().ok_or(FAILURE)?;
        let (tx, events) = mpsc::sync_channel(128);
        let reader = thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Ok(line) = crate::bridge::read_line(&mut reader, 512 * 1024) {
                if line.is_empty() {
                    break;
                }
                let Ok(value) = serde_json::from_slice::<Value>(&line) else {
                    break;
                };
                // Do not block process teardown on a full event queue.
                if tx.try_send(value).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input,
            events,
            pending: VecDeque::new(),
            reader: Some(reader),
            cancel,
            deadline: Instant::now() + Duration::from_secs(240),
            next: 0,
            retry: Retry::Never,
        })
    }
    fn send(&mut self, value: Value) -> Result<()> {
        writeln!(self.input, "{value}").map_err(failed)?;
        self.input.flush().map_err(failed)
    }
    fn receive(&mut self) -> Result<Value> {
        loop {
            if self.cancel.load(Ordering::SeqCst) {
                return Err("Analysis stopped.".into());
            }
            if Instant::now() > self.deadline {
                self.retry = Retry::Transient;
                record("codex.timeout", &[]);
                return Err("The request timed out. Try again.".into());
            }
            match self.events.recv_timeout(Duration::from_millis(100)) {
                Ok(event) => {
                    // ME inbox does not grant model-initiated tools, approvals,
                    // external credentials or user interactions.
                    if event.get("method").is_some() && event.get("id").is_some() {
                        self.send(json!({"id":event["id"],"error":{"code":-32601,"message":"Tool execution is disabled for ME inbox"}}))?;
                        return Err("The model requested an unsupported action.".into());
                    }
                    return Ok(event);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => {
                    self.retry = Retry::Transient;
                    let code = self.child.try_wait().ok().flatten().and_then(|s| s.code());
                    record(
                        "codex.disconnected",
                        &[F::Integer("exit_code", code.unwrap_or(-1) as i64)],
                    );
                    return Err(match code {
                        Some(127)=>"Codex couldn't find its runtime. Update Codex or use the native binary.".into(),
                        Some(code)=>format!("Codex closed unexpectedly (exit code {code}). Check the installation."),
                        None=>"The local connection closed. Please reconnect.".into(),
                    });
                }
            }
        }
    }
    fn rpc(&mut self, method: &'static str, params: Value) -> Result<Value> {
        let started = Instant::now();
        record("codex.rpc.started", &[F::Label("method", method)]);
        self.next += 1;
        let id = self.next;
        self.send(json!({"id":id,"method":method,"params":params}))?;
        loop {
            let event = self.receive()?;
            if event.get("id") == Some(&json!(id)) {
                if event.get("error").is_some() {
                    let code = event
                        .pointer("/error/code")
                        .and_then(Value::as_i64)
                        .unwrap_or(0);
                    record(
                        "codex.rpc.failed",
                        &[F::Label("method", method), F::Integer("code", code)],
                    );
                    return Err(format!(
                        "Codex couldn't process {method} (error {code}). Update ME. and Codex, then try again."
                    ));
                }
                record(
                    "codex.rpc.finished",
                    &[
                        F::Label("method", method),
                        F::Count("duration_ms", started.elapsed().as_millis() as u64),
                    ],
                );
                return event.get("result").cloned().ok_or_else(|| FAILURE.into());
            }
            if self.pending.len() >= 256 {
                return Err(FAILURE.into());
            }
            self.pending.push_back(event);
        }
    }
    fn next_event(&mut self) -> Result<Value> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err("Connection stopped.".into());
        }
        if let Some(e) = self.pending.pop_front() {
            Ok(e)
        } else {
            self.receive()
        }
    }
}
/// No persistent "configured" flag: readiness is established against the live App Server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetupIssue {
    SignInRequired,
    WrongAccount,
    ModelUnavailable,
    Connection(String),
}
impl SetupIssue {
    pub fn message(&self) -> String {
        match self {
            Self::SignInRequired => "Connect your ChatGPT account to use ME.".into(),
            Self::WrongAccount => "Sign in with your ChatGPT subscription to continue.".into(),
            Self::ModelUnavailable => {
                "The analysis model isn't available for this account. Check your subscription and update Codex."
                    .into()
            }
            Self::Connection(message) => message.clone(),
        }
    }
}
type SetupResult<T> = std::result::Result<T, SetupIssue>;

fn initialize(session: &mut Session) -> SetupResult<()> {
    session.rpc("initialize",json!({"clientInfo":{"name":"me_inbox","title":"ME Inbox","version":"0.1.0"},"capabilities":{"experimentalApi":true}})).map_err(SetupIssue::Connection)?;
    session
        .send(json!({"method":"initialized"}))
        .map_err(SetupIssue::Connection)
}
fn verify_configuration(session: &mut Session) -> SetupResult<()> {
    let account = session
        .rpc("account/read", json!({"refreshToken":true}))
        .map_err(|_| {
            SetupIssue::Connection(
                "Couldn't verify sign-in. Check your connection or sign in again.".into(),
            )
        })?;
    match account.pointer("/account/type").and_then(Value::as_str) {
        None => return Err(SetupIssue::SignInRequired),
        Some("chatgpt") => {}
        _ => return Err(SetupIssue::WrongAccount),
    }
    let mut cursor = Value::Null;
    let mut available = false;
    for _ in 0..10 {
        let models = session
            .rpc("model/list", json!({"cursor":cursor,"limit":100}))
            .map_err(SetupIssue::Connection)?;
        if models
            .get("data")
            .and_then(Value::as_array)
            .is_some_and(|models| {
                models
                    .iter()
                    .any(|m| m.get("model").and_then(Value::as_str) == Some(INBOX_MODEL))
            })
        {
            available = true;
            break;
        }
        cursor = models.get("nextCursor").cloned().unwrap_or(Value::Null);
        if cursor.is_null() {
            break;
        }
    }
    if !available {
        return Err(SetupIssue::ModelUnavailable);
    }
    // This authenticated service call checks connectivity without creating a model turn.
    // Exhausted quota is still a configured account and does not block access to the vault.
    session
        .rpc("account/rateLimits/read", json!({}))
        .map_err(|_| {
            SetupIssue::Connection(
                "Couldn't connect to ChatGPT. Check your connection or sign in again.".into(),
            )
        })?;
    Ok(())
}

pub fn check_configuration(
    home: &Path,
    cancel: Arc<AtomicBool>,
    mut progress: impl FnMut(Progress),
) -> SetupResult<()> {
    setup_with_binary(home, false, cancel, &mut progress, &executable())
}
/// Called only from the setup screen's explicit connect action; never sends document text.
pub fn configure(
    home: &Path,
    cancel: Arc<AtomicBool>,
    mut progress: impl FnMut(Progress),
) -> SetupResult<()> {
    setup_with_binary(home, true, cancel, &mut progress, &executable())
}
fn setup_with_binary(
    home: &Path,
    login: bool,
    cancel: Arc<AtomicBool>,
    progress: &mut impl FnMut(Progress),
    binary: &Path,
) -> SetupResult<()> {
    let scratch = tempfile::Builder::new()
        .prefix("me-codex-setup-")
        .tempdir()
        .map_err(|e| SetupIssue::Connection(failed(e)))?;
    let mut session = Session::start(home, scratch.path(), cancel.clone(), binary)
        .map_err(SetupIssue::Connection)?;
    session.deadline = Instant::now() + Duration::from_secs(30);
    initialize(&mut session)?;
    if login {
        let response = session
            .rpc("account/login/start", json!({"type":"chatgpt"}))
            .map_err(SetupIssue::Connection)?;
        let login_id = response
            .get("loginId")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| SetupIssue::Connection("Couldn't start sign-in. Try again.".into()))?
            .to_string();
        let url = response
            .get("authUrl")
            .and_then(Value::as_str)
            .filter(|url| {
                url.len() <= 4096
                    && !url.chars().any(char::is_control)
                    && (url.starts_with("https://auth.openai.com/")
                        || url.starts_with("https://chatgpt.com/"))
            })
            .ok_or_else(|| {
                SetupIssue::Connection("Couldn't open the sign-in page. Try again.".into())
            })?;
        if cancel.load(Ordering::SeqCst) {
            return Err(SetupIssue::Connection("Sign-in stopped.".into()));
        }
        progress(Progress::LoginUrl(url.into()));
        session.deadline = Instant::now() + Duration::from_secs(600);
        loop {
            let event = session.next_event().map_err(SetupIssue::Connection)?;
            if event.get("method").and_then(Value::as_str) == Some("account/login/completed")
                && event.pointer("/params/loginId").and_then(Value::as_str) == Some(&login_id)
            {
                if event.pointer("/params/success") != Some(&json!(true)) {
                    return Err(SetupIssue::Connection(
                        "Sign-in wasn't completed. Try again.".into(),
                    ));
                }
                break;
            }
        }
        progress(Progress::Message("Signed in. Checking connection…".into()));
        session.deadline = Instant::now() + Duration::from_secs(30);
    }
    verify_configuration(&mut session)?;
    if cancel.load(Ordering::SeqCst) {
        return Err(SetupIssue::Connection("Connection stopped.".into()));
    }
    Ok(())
}

fn retry_for_error(error: &Value) -> Retry {
    match error.get("codexErrorInfo").and_then(Value::as_str) {
        Some("contextWindowExceeded") => Retry::Smaller,
        Some("serverOverloaded" | "internalServerError") => Retry::Transient,
        _ => Retry::Never,
    }
}
fn turn_error(error: &Value) -> String {
    let code = match error.get("codexErrorInfo").and_then(Value::as_str) {
        Some("usageLimitExceeded") => "usageLimitExceeded",
        Some("sessionBudgetExceeded") => "sessionBudgetExceeded",
        Some("unauthorized") => "unauthorized",
        Some("contextWindowExceeded") => "contextWindowExceeded",
        Some("serverOverloaded") => "serverOverloaded",
        Some("internalServerError") => "internalServerError",
        Some("badRequest") => "badRequest",
        _ => "unknown",
    };
    record("codex.turn.failed", &[F::Label("code", code)]);
    match error.get("codexErrorInfo").and_then(Value::as_str) {
        Some("usageLimitExceeded" | "sessionBudgetExceeded") => {
            "Your ChatGPT usage limit has been reached. Try again after it resets.".into()
        }
        Some("unauthorized") => "Your sign-in has expired. Reconnect ChatGPT.".into(),
        Some("contextWindowExceeded") => "This document is too large for one request.".into(),
        Some("serverOverloaded" | "internalServerError") => {
            "ChatGPT is temporarily unavailable. Try again later.".into()
        }
        Some("badRequest") => "The request was declined. Update ME. and Codex.".into(),
        _ => FAILURE.into(),
    }
}

pub fn output_schema() -> Value {
    let text = json!({"type":"string"});
    json!({"type":"object","additionalProperties":false,"required":["facts"],"properties":{"facts":{"type":"array","items":{"type":"object","additionalProperties":false,"required":["property","value","segment_id","quote","subject_quote","context_quote"],"properties":{"property":text,"value":text,"segment_id":text,"quote":text,"subject_quote":text,"context_quote":text}}}}})
}
pub fn extract(
    home: &Path,
    input: &ExtractionInput,
    cancel: Arc<AtomicBool>,
    mut progress: impl FnMut(Progress),
) -> Result<ExtractionOutput> {
    extract_with_binary(home, input, cancel, &mut progress, &executable())
}
pub fn extract_document(
    home: &Path,
    input: &ExtractionInput,
    cancel: Arc<AtomicBool>,
    mut progress: impl FnMut(Progress),
    checkpoints: &mut impl Checkpoints,
) -> Result<ExtractionReport> {
    pipeline::run(
        home,
        input,
        cancel,
        &mut progress,
        &executable(),
        checkpoints,
    )
}
fn extract_with_binary(
    home: &Path,
    input: &ExtractionInput,
    cancel: Arc<AtomicBool>,
    progress: &mut impl FnMut(Progress),
    binary: &Path,
) -> Result<ExtractionOutput> {
    pipeline::run(
        home,
        input,
        cancel,
        progress,
        binary,
        &mut pipeline::NoCheckpoints,
    )
    .map(|report| report.output)
}

const BATCH_TEXT_BYTES: usize = 12_000;
fn extraction_batches(input: &ExtractionInput) -> Result<Vec<ExtractionInput>> {
    let total: usize = input.segments.iter().map(|s| s.text.len()).sum();
    if input.segments.is_empty() || total > me_core::MAX_DOCUMENT_TEXT {
        return Err("No supported document text found (maximum 8 MiB).".into());
    }
    let mut batches = Vec::new();
    let mut batch = input.with_segments(Vec::new());
    let mut bytes = 0;
    for segment in &input.segments {
        // Canonical vault segments are <= 1600 bytes. Never cut quotations,
        // silently truncate, or change the IDs used for evidence validation.
        if segment.text.len() > BATCH_TEXT_BYTES {
            return Err("A source excerpt is too large. Import the document again.".into());
        }
        if bytes + segment.text.len() > BATCH_TEXT_BYTES {
            let overlap = batch
                .segments
                .last()
                .cloned()
                .filter(|s| s.text.len() + segment.text.len() <= BATCH_TEXT_BYTES);
            batches.push(batch);
            batch = input.with_segments(overlap.into_iter().collect());
            bytes = batch.segments.iter().map(|s| s.text.len()).sum();
        }
        bytes += segment.text.len();
        batch.segments.push(segment.clone());
    }
    if !batch.segments.is_empty() {
        batches.push(batch);
    }
    Ok(batches)
}

fn request_model(
    session: &mut Session,
    scratch: &Path,
    progress: &mut impl FnMut(Progress),
    instructions: &str,
    prompt: Value,
    schema: Value,
) -> Result<Value> {
    session.deadline = Instant::now() + Duration::from_secs(240);
    let thread=session.rpc("thread/start",json!({"model":INBOX_MODEL,"modelProvider":"openai","allowProviderModelFallback":false,"cwd":scratch,"ephemeral":true,"permissions":"me-inbox","approvalPolicy":"never","baseInstructions":instructions,"developerInstructions":"Return only the constrained extraction object.","environments":[],"selectedCapabilityRoots":[]}))?;
    let thread_id = thread
        .pointer("/thread/id")
        .and_then(Value::as_str)
        .ok_or(FAILURE)?
        .to_string();
    let prompt = serde_json::to_string(&prompt).map_err(failed)?;
    let turn=session.rpc("turn/start",json!({"threadId":thread_id,"model":INBOX_MODEL,"effort":INBOX_EFFORT,"input":[{"type":"text","text":prompt}],"outputSchema":schema,"approvalPolicy":"never","permissions":"me-inbox","environments":[]}))?;
    let turn_id = turn
        .pointer("/turn/id")
        .and_then(Value::as_str)
        .ok_or(FAILURE)?
        .to_string();
    await_json(session, &thread_id, &turn_id, progress)
}

fn await_json(
    session: &mut Session,
    thread_id: &str,
    turn_id: &str,
    progress: &mut impl FnMut(Progress),
) -> Result<Value> {
    let mut answer = None;
    loop {
        let event = session.next_event()?;
        let method = event.get("method").and_then(Value::as_str).unwrap_or("");
        if method == "account/updated"
            && event.pointer("/params/authMode") != Some(&json!("chatgpt"))
        {
            progress(Progress::SetupRequired(SetupIssue::SignInRequired));
            return Err(SetupIssue::SignInRequired.message());
        }
        if method == "model/rerouted" {
            return Err("The model changed unexpectedly. Analysis was stopped.".into());
        }
        if event.pointer("/params/threadId").and_then(Value::as_str) != Some(thread_id) {
            continue;
        }
        if method == "error"
            && event.pointer("/params/turnId").and_then(Value::as_str) == Some(turn_id)
            && event.pointer("/params/willRetry") == Some(&json!(false))
        {
            if event
                .pointer("/params/error/codexErrorInfo")
                .and_then(Value::as_str)
                == Some("unauthorized")
            {
                progress(Progress::SetupRequired(SetupIssue::SignInRequired));
            }
            session.retry = retry_for_error(&event["params"]["error"]);
            return Err(turn_error(&event["params"]["error"]));
        }
        if method == "item/completed"
            && event.pointer("/params/turnId").and_then(Value::as_str) == Some(turn_id)
        {
            let item = &event["params"]["item"];
            if item["type"] == "agentMessage"
                && (item["phase"].is_null() || item["phase"] == "final_answer")
            {
                answer = item.get("text").and_then(Value::as_str).map(str::to_owned);
            }
        }
        if method == "turn/completed"
            && event.pointer("/params/turn/id").and_then(Value::as_str) == Some(turn_id)
        {
            if event.pointer("/params/turn/status").and_then(Value::as_str) != Some("completed") {
                session.retry = retry_for_error(&event["params"]["turn"]["error"]);
                return Err(turn_error(&event["params"]["turn"]["error"]));
            }
            let answer = answer.ok_or_else(|| {
                session.retry = Retry::InvalidOutput;
                record("codex.answer.missing", &[]);
                "No usable answer was returned. Try again.".to_string()
            })?;
            if answer.len() > 192_000 {
                session.retry = Retry::Smaller;
                record(
                    "codex.answer.too_large",
                    &[F::Count("bytes", answer.len() as u64)],
                );
                return Err("The answer exceeded the size limit. Try again.".into());
            }
            return serde_json::from_str::<Value>(&answer).map_err(|_| {
                session.retry = Retry::InvalidOutput;
                record("codex.answer.invalid_schema", &[]);
                "The response had an unexpected format. Try again.".into()
            });
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    pub(super) fn fake(path: &Path, mode: &str) {
        let script=r##"#!/usr/bin/env python3
import sys,json,re,pathlib
mode='MODE'
turns=0
def send(x):
 print(json.dumps(x),flush=True)
for line in sys.stdin:
 r=json.loads(line);method=r.get('method');i=r.get('id');p=r.get('params',{})
 if i is None: continue
 if method=='initialize': result={'capabilities':{}}
 elif method=='account/read': result={'account':{'type':'chatgpt'}}
 elif method=='model/list': result={'data':[{'model':'gpt-5.6-sol'}],'nextCursor':None}
 elif method=='thread/start':
  assert p['ephemeral'] and p['permissions']=='me-inbox' and 'sandbox' not in p and p['approvalPolicy']=='never'
  result={'thread':{'id':'thread-test'}}
 elif method=='turn/start':
  assert p['model']=='gpt-5.6-sol' and p['effort']=='medium'
  assert p['permissions']=='me-inbox' and 'sandboxPolicy' not in p
  assert p['outputSchema']['additionalProperties']==False
  if mode=='tool':
   send({'id':999,'method':'item/tool/call','params':{'tool':'shell'}})
   response=json.loads(sys.stdin.readline());assert 'error' in response
   break
  source=json.loads(p['input'][0]['text'][p['input'][0]['text'].find('{'):])
  stage=source.get('stage','extract')
  if stage=='classify':
   text=json.dumps({'document_type':'generic','country':'unknown','language':'de','layout_notes':'synthetic','fields_to_check':['tax ID']})
   send({'id':i,'result':{'turn':{'id':'turn-test'}}})
   send({'method':'item/completed','params':{'threadId':'thread-test','turnId':'turn-test','item':{'type':'agentMessage','phase':'final_answer','text':text}}})
   send({'method':'turn/completed','params':{'threadId':'thread-test','turn':{'id':'turn-test','status':'completed'}}})
   continue
  if stage in ['extract','repair']: turns+=1
  if mode=='quota' or (mode=='late_failure' and turns==2):
   send({'id':i,'result':{'turn':{'id':'turn-test'}}})
   send({'method':'error','params':{'threadId':'thread-test','turnId':'turn-test','willRetry':False,'error':{'codexErrorInfo':'usageLimitExceeded','message':'SYNTHETIC private provider details'}}})
   continue
  if mode=='protocol':
   send({'id':i,'error':{'code':-32600,'message':'SYNTHETIC private provider details'}})
   continue
  if mode=='large' and len(source['segments'])>2:
   send({'id':i,'result':{'turn':{'id':'turn-test'}}})
   send({'method':'error','params':{'threadId':'thread-test','turnId':'turn-test','willRetry':False,'error':{'codexErrorInfo':'contextWindowExceeded'}}})
   continue
  if mode=='transient' and not pathlib.Path(sys.argv[0]+'.retried').exists():
   pathlib.Path(sys.argv[0]+'.retried').write_text('synthetic retry')
   send({'id':i,'result':{'turn':{'id':'turn-test'}}})
   send({'method':'error','params':{'threadId':'thread-test','turnId':'turn-test','willRetry':False,'error':{'codexErrorInfo':'serverOverloaded'}}})
   continue
  text='not JSON' if mode=='invalid' else '{"facts":[]}'
  if stage in ['extract','repair'] and mode in ['batches','late_failure','large','transient','bad_evidence','repair','bad_subject']:
   assert sum(len(s['text'].encode('utf-8')) for s in source['segments'])<=12000
   facts=[]
   for s in source['segments']:
    value=re.search(r'Steuer-ID: ([0-9]+)',s['text']).group(1)
    facts.append({'property':'person.tax_id','value':value,'segment_id':s['segment_id'],'quote':value,'subject_quote':'Erika Beispiel'})
   if mode=='bad_evidence' or (mode=='repair' and turns==1): facts[-1]['quote']='invented '+facts[-1]['value']
   if mode=='bad_subject': facts[-1]['subject_quote']='Unknown Person'
   text=json.dumps({'facts':facts})
   with open(sys.argv[0]+'.calls','a') as log: log.write(json.dumps([s['segment_id'] for s in source['segments']])+'\n')

  if mode=='audit_missing':
   assert source.get('document_profile',{}).get('document_type')=='generic'
   assert stage in ['extract','audit']
   facts=[]
   if stage=='audit':
    assert source['previous_facts']['facts']==[]
    for index in range(24):
     value=f'{index},00 EUR'
     facts.append({'property':f'document.Zulage {index}','value':value,'segment_id':source['segments'][0]['segment_id'],'quote':value,'subject_quote':'Erika Beispiel','context_quote':'August 2026'})
   text=json.dumps({'facts':facts})
  if mode=='audit_failure' and stage=='audit':
   send({'id':i,'error':{'code':-32600,'message':'synthetic audit failure'}})
   continue
  send({'method':'item/completed','params':{'threadId':'thread-test','turnId':'turn-test','item':{'type':'agentMessage','phase':'final_answer','text':text}}})
  result={'turn':{'id':'turn-test'}}
  send({'id':i,'result':result})
  send({'method':'turn/completed','params':{'threadId':'thread-test','turn':{'id':'turn-test','status':'completed'}}})
  continue
 else: result={}
 send({'id':i,'result':result})
"##.replace("MODE",mode);
        fs::write(path, script).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    fn run(mode: &str) -> Result<ExtractionOutput> {
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("fake-codex");
        fake(&bin, mode);
        let input = ExtractionInput {
            run_id: "test".into(),
            source_id: "test".into(),
            title: "Synthetic".into(),
            segments: vec![me_core::SourceText {
                segment_id: "segment-test".into(),
                ordinal: 1,
                text: "SYNTHETIC".into(),
            }],
        };
        extract_with_binary(
            &temp.path().join("home"),
            &input,
            Arc::new(AtomicBool::new(false)),
            &mut |_| {},
            &bin,
        )
    }
    #[test]
    fn structured_completion_handles_events_before_rpc_response() {
        assert!(run("ok").unwrap().facts.is_empty());
    }
    #[test]
    fn provider_and_protocol_failures_are_actionable_without_raw_error_details() {
        let quota = run("quota").unwrap_err();
        assert!(quota.contains("usage limit"));
        assert!(!quota.contains("private"));
        let protocol = run("protocol").unwrap_err();
        assert!(protocol.contains("turn/start"));
        assert!(protocol.contains("-32600"));
        assert!(!protocol.contains("private"));
    }
    #[test]
    fn inbox_permissions_only_allow_minimal_platform_and_scratch_reads() {
        let temp = tempfile::tempdir().unwrap();
        let cmd = Session::command(
            &temp.path().join("home"),
            temp.path(),
            Path::new("synthetic"),
        )
        .unwrap();
        let args: Vec<_> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.contains(&"default_permissions=\"me-inbox\"".into()));
        assert!(args.contains(&"permissions.me-inbox.network.enabled=false".into()));
        assert!(args.contains(&"features.plugins=false".into()));
        assert!(args.contains(&"features.remote_plugin=false".into()));
        let profile = args
            .iter()
            .find(|a| a.starts_with("permissions.me-inbox.filesystem="))
            .unwrap();
        assert!(profile.contains("\":root\"=\"deny\""));
        assert!(profile.contains("\":minimal\"=\"read\""));
        assert!(!profile.contains("write"));
        assert!(profile.contains(&temp.path().to_string_lossy().to_string()));
    }
    #[test]
    fn tool_requests_are_denied_and_invalid_json_is_not_accepted() {
        assert!(run("tool").unwrap_err().contains("unsupported action"));
        assert!(run("invalid").is_err());
    }
}

#[cfg(test)]
#[path = "codex_setup_tests.rs"]
mod setup_tests;

#[cfg(test)]
#[path = "codex_batch_tests.rs"]
mod batch_tests;
