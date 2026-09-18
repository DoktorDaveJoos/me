use me_core::{ReadScope, Vault};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::{
        fs::{MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

const MAX_REQUEST: u64 = 32 * 1024;
const MAX_RESPONSE: u64 = 256 * 1024;
const DESCRIPTOR: &str = "codex-bridge.json";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Connection {
    version: u32,
    socket: PathBuf,
    token: String,
}
pub struct BridgeServer {
    enabled: Arc<AtomicBool>,
    socket: PathBuf,
    worker: Option<thread::JoinHandle<()>>,
}
impl BridgeServer {
    /// Start on a background worker after explicit UI authorization. The scope
    /// is a snapshot, and newly imported sources require a new grant.
    pub fn start(
        root: &Path,
        vault: Arc<Mutex<Option<Vault>>>,
        scope: ReadScope,
    ) -> io::Result<Self> {
        let dir = tempfile::Builder::new().prefix("me-mcp-").tempdir()?;
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700))?;
        let socket = dir.path().join("s");
        let listener = UnixListener::bind(&socket)?;
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
        let info = Connection {
            version: 1,
            socket: socket.clone(),
            token: format!("{}{}", uuid::Uuid::new_v4(), uuid::Uuid::new_v4()),
        };
        let descriptor = root.join(DESCRIPTOR);
        if descriptor
            .symlink_metadata()
            .is_ok_and(|m| !m.file_type().is_file())
        {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let mut file = tempfile::NamedTempFile::new_in(root)?;
        file.write_all(&serde_json::to_vec(&info)?)?;
        file.as_file().sync_all()?;
        file.persist(&descriptor).map_err(|e| e.error)?;
        let enabled = Arc::new(AtomicBool::new(true));
        let running = enabled.clone();
        let worker = thread::spawn(move || {
            let _dir = dir;
            for stream in listener.incoming() {
                if !running.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(mut stream) = stream else { break };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                let result = (|| -> io::Result<Value> {
                    let request = read_line(&mut BufReader::new(&stream), MAX_REQUEST)?;
                    let request: Value = serde_json::from_slice(&request)?;
                    if request.get("token").and_then(Value::as_str) != Some(&info.token) {
                        return Err(io::ErrorKind::PermissionDenied.into());
                    }
                    if !running.load(Ordering::SeqCst) {
                        return Ok(unavailable());
                    }
                    let guard = vault.lock().map_err(|_| io::ErrorKind::Other)?;
                    let Some(vault) = guard.as_ref() else {
                        return Ok(unavailable());
                    };
                    let name = request.get("name").and_then(Value::as_str).unwrap_or("");
                    let args = request.get("arguments").cloned().unwrap_or(json!({}));
                    let result = dispatch(vault, &scope, name, args);
                    if !running.load(Ordering::SeqCst) {
                        return Ok(unavailable());
                    }
                    Ok(result)
                })()
                .unwrap_or_else(
                    |_| json!({"error":"access_denied","message":"ME. access hasn't been enabled."}),
                );
                // Revocation is checked immediately before emitting a result.
                let result = if running.load(Ordering::SeqCst) {
                    result
                } else {
                    unavailable()
                };
                let _ = writeln!(stream, "{result}");
            }
            // Remove only this session's descriptor, never a newer grant.
            if fs::read(&descriptor)
                .ok()
                .and_then(|b| serde_json::from_slice::<Connection>(&b).ok())
                .is_some_and(|c| c.token == info.token)
            {
                let _ = fs::remove_file(descriptor);
            }
        });
        Ok(Self {
            enabled,
            socket,
            worker: Some(worker),
        })
    }
    /// Immediate in-memory revocation; cleanup and joining happen on a worker.
    pub fn revoke(&self) {
        self.enabled.store(false, Ordering::SeqCst);
    }
}
impl Drop for BridgeServer {
    fn drop(&mut self) {
        self.revoke();
        let _ = UnixStream::connect(&self.socket);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
pub(crate) fn read_line(reader: &mut impl BufRead, max: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(max + 1).read_until(b'\n', &mut bytes)?;
    if bytes.len() as u64 > max || (!bytes.is_empty() && !bytes.ends_with(b"\n")) {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(bytes)
}
pub(crate) fn unavailable() -> Value {
    json!({"error":"vault_unavailable","message":"Open ME., unlock the vault, and enable connected access."})
}
pub fn bridge_call(root: &Path, name: &str, args: Value) -> Value {
    let result = (|| -> io::Result<Value> {
        let path = root.join(DESCRIPTOR);
        let meta = fs::symlink_metadata(&path)?;
        let root_meta = fs::symlink_metadata(root)?;
        if !meta.file_type().is_file()
            || !root_meta.file_type().is_dir()
            || meta.mode() & 0o077 != 0
            || root_meta.mode() & 0o077 != 0
            || meta.uid() != root_meta.uid()
            || meta.len() > 4096
        {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let info: Connection = serde_json::from_slice(&fs::read(path)?)?;
        if info.version != 1 || !info.socket.is_absolute() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let mut stream = UnixStream::connect(&info.socket)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        writeln!(
            stream,
            "{}",
            json!({"token":info.token,"name":name,"arguments":args})
        )?;
        serde_json::from_slice(&read_line(&mut BufReader::new(stream), MAX_RESPONSE)?)
            .map_err(Into::into)
    })();
    result.unwrap_or_else(|_| unavailable())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FactsArgs {
    fields: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    query: String,
    #[serde(default = "default_limit")]
    limit: usize,
}
fn default_limit() -> usize {
    10
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceArgs {
    segment_id: String,
}
fn dispatch(vault: &Vault, scope: &ReadScope, name: &str, args: Value) -> Value {
    let result = (|| -> me_core::Result<Value> {
        match name {
            "me_status" if args == json!({}) => Ok(
                json!({"status":"ready","access":"read_only","coverage":"shared_sources_snapshot","shared_sources":scope.len(),"fields":me_core::STANDARD_FIELDS}),
            ),
            "me_facts_get" => {
                let a: FactsArgs = serde_json::from_value(args)
                    .map_err(|_| me_core::Error::Validation("Invalid tool parameters."))?;
                vault.facts_get(scope, &a.fields)
            }
            "me_documents_search" => {
                let a: SearchArgs = serde_json::from_value(args)
                    .map_err(|_| me_core::Error::Validation("Invalid tool parameters."))?;
                vault.documents_search(scope, &a.query, a.limit)
            }
            "me_evidence_read" => {
                let a: EvidenceArgs = serde_json::from_value(args)
                    .map_err(|_| me_core::Error::Validation("Invalid tool parameters."))?;
                vault.evidence_read(scope, &a.segment_id)
            }
            _ => Err(me_core::Error::Validation("Unknown or unavailable tool.")),
        }
    })();
    result.unwrap_or_else(|e| json!({"error":"request_failed","message":e.to_string()}))
}
