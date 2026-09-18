//! Local metadata-only diagnostics. No dynamic strings in the recording API:
//! document names, text, provider payloads and credentials cannot be fields.
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

const FILE_LIMIT: u64 = 1024 * 1024;
const HISTORY: usize = 200;
pub enum Field {
    Label(&'static str, &'static str),
    Count(&'static str, u64),
    Integer(&'static str, i64),
    Flag(&'static str, bool),
}
struct Logger {
    sender: mpsc::SyncSender<String>,
    recent: Mutex<VecDeque<String>>,
    path: PathBuf,
    build: &'static str,
    status: Mutex<&'static str>,
    dropped: AtomicU64,
}
static LOGGER: OnceLock<Logger> = OnceLock::new();

/// Starts I/O on a dedicated worker, never the UI thread. A bounded queue and
/// rotated files prevent diagnostics from blocking processing or filling disk.
pub fn init(path: PathBuf, build: &'static str) {
    let (sender, receiver) = mpsc::sync_channel::<String>(512);
    if LOGGER
        .set(Logger {
            sender,
            recent: Mutex::new(VecDeque::new()),
            path: path.clone(),
            build,
            status: Mutex::new("starting"),
            dropped: AtomicU64::new(0),
        })
        .is_err()
    {
        return;
    }
    std::thread::spawn(move || {
        let mut sink = Sink::open(&path, FILE_LIMIT);
        set_status(if sink.is_ok() {
            "active"
        } else {
            "file_unavailable"
        });
        for line in receiver {
            if let Ok(writer) = &mut sink
                && writer.write(&line).is_err()
            {
                sink = Err(io::Error::other("diagnostic write failed"));
                set_status("file_unavailable");
            }
        }
    });
    record(
        "app.started",
        &[
            Field::Label("version", env!("CARGO_PKG_VERSION")),
            Field::Label("build", build),
            Field::Label("os", std::env::consts::OS),
        ],
    );
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Deliberately omit the panic message, which can contain user data.
        record("app.panic", &[]);
        previous(info);
    }));
}
fn set_status(value: &'static str) {
    if let Some(log) = LOGGER.get()
        && let Ok(mut status) = log.status.lock()
    {
        *status = value;
    }
}
pub fn directory() -> Option<PathBuf> {
    LOGGER.get().map(|log| log.path.clone())
}
pub fn record(event: &'static str, fields: &[Field]) {
    let Some(log) = LOGGER.get() else {
        return;
    };
    let mut value = json!({"time_unix_ms": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64, "pid": std::process::id(), "event": event});
    for field in fields {
        let (key, data) = match field {
            Field::Label(k, v) => (*k, Value::from(*v)),
            Field::Count(k, v) => (*k, Value::from(*v)),
            Field::Integer(k, v) => (*k, Value::from(*v)),
            Field::Flag(k, v) => (*k, Value::from(*v)),
        };
        value[key] = data;
    }
    let line = value.to_string();
    if let Ok(mut recent) = log.recent.lock() {
        if recent.len() == HISTORY {
            recent.pop_front();
        }
        recent.push_back(line.clone());
    }
    if log.sender.try_send(line).is_err() {
        log.dropped.fetch_add(1, Ordering::Relaxed);
    }
}
/// Snapshot of this process only. Reading never waits for disk or a provider.
pub fn report() -> String {
    let Some(log) = LOGGER.get() else {
        return "ME. diagnostics: unavailable".into();
    };
    let status = log.status.lock().map(|s| *s).unwrap_or("unavailable");
    let lines = log
        .recent
        .lock()
        .map(|s| s.iter().cloned().collect::<Vec<_>>().join("\n"))
        .unwrap_or_default();
    format!(
        "ME. diagnostics\nversion={}\nbuild={}\nfile_status={status}\ndropped_events={}\n{lines}\n",
        env!("CARGO_PKG_VERSION"),
        log.build,
        log.dropped.load(Ordering::Relaxed)
    )
}

struct Sink {
    directory: PathBuf,
    file: File,
    bytes: u64,
    limit: u64,
}
impl Sink {
    fn open(directory: &Path, limit: u64) -> io::Result<Self> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)
            .or_else(|e| {
                if e.kind() == io::ErrorKind::AlreadyExists {
                    Ok(())
                } else {
                    Err(e)
                }
            })?;
        let meta = fs::symlink_metadata(directory)?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(io::Error::other("invalid log directory"));
        }
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
        let file = Self::open_file(directory)?;
        let bytes = file.metadata()?.len();
        Ok(Self {
            directory: directory.into(),
            file,
            bytes,
            limit,
        })
    }
    fn open_file(directory: &Path) -> io::Result<File> {
        let path = directory.join("diagnostics.jsonl");
        match fs::symlink_metadata(&path) {
            Ok(meta) if !meta.is_file() || meta.file_type().is_symlink() => {
                return Err(io::Error::other("invalid log file"));
            }
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        Ok(file)
    }
    fn write(&mut self, line: &str) -> io::Result<()> {
        if self.bytes > 0 && self.bytes + line.len() as u64 + 1 > self.limit {
            for index in (1..=3).rev() {
                let from = self.directory.join(if index == 1 {
                    "diagnostics.jsonl".into()
                } else {
                    format!("diagnostics.{}.jsonl", index - 1)
                });
                let to = self.directory.join(format!("diagnostics.{index}.jsonl"));
                if let Err(e) = fs::rename(from, to)
                    && e.kind() != io::ErrorKind::NotFound
                {
                    return Err(e);
                }
            }
            self.file = Self::open_file(&self.directory)?;
            self.bytes = 0;
        }
        writeln!(self.file, "{line}")?;
        self.file.flush()?;
        self.bytes += line.len() as u64 + 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn logs_rotate_with_private_permissions_and_remain_json_lines() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("logs");
        let mut sink = Sink::open(&root, 100).unwrap();
        for i in 0..100 {
            sink.write(&json!({"event":"synthetic", "count": i}).to_string())
                .unwrap();
        }
        assert_eq!(fs::read_dir(&root).unwrap().count(), 4);
        assert_eq!(
            fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        for entry in fs::read_dir(&root).unwrap() {
            let path = entry.unwrap().path();
            let metadata = fs::metadata(&path).unwrap();
            assert!(metadata.len() <= 100);
            assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
            for line in fs::read_to_string(path).unwrap().lines() {
                serde_json::from_str::<Value>(line).unwrap();
            }
        }
    }
    #[test]
    fn does_not_follow_log_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let other = temp.path().join("other");
        fs::write(&other, "untouched").unwrap();
        let root = temp.path().join("logs");
        fs::create_dir(&root).unwrap();
        std::os::unix::fs::symlink(&other, root.join("diagnostics.jsonl")).unwrap();
        assert!(Sink::open(&root, 100).is_err());
        assert_eq!(fs::read_to_string(other).unwrap(), "untouched");
    }
}
