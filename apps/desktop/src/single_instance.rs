//! One running ME. per app identity.
//!
//! The first process listens on a Unix socket derived from its vault directory,
//! which is already separate for ME., ME Dev and ME Preview. A later launch sends
//! `show` to it and exits. Only that literal command crosses the socket.

use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_channel::mpsc::UnboundedSender;

const SHOW: &str = "show";

pub enum Claim {
    /// This process is the instance; forward requests with [`serve`].
    Primary(UnixListener),
    /// Another instance was asked to show itself; this process should exit.
    HandedOff,
}

/// A short socket path in the per-user temporary directory. Unix socket paths
/// are limited to about 100 bytes, which a vault directory can exceed.
pub fn socket_path(vault_dir: &Path) -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .or_else(|| std::env::var_os("TMPDIR"))
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(std::env::temp_dir);
    base.join(format!(
        "me-{:016x}.sock",
        fnv1a(vault_dir.as_os_str().as_encoded_bytes())
    ))
}

/// Stable across builds, unlike `DefaultHasher`, so different versions of the
/// same app identity still find each other.
fn fnv1a(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 14_695_981_039_346_656_037;
    const PRIME: u64 = 1_099_511_628_211;
    bytes.iter().fold(OFFSET_BASIS, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(PRIME)
    })
}

pub fn claim(path: &Path) -> std::io::Result<Claim> {
    match bind(path) {
        Ok(listener) => Ok(Claim::Primary(listener)),
        Err(error) if error.kind() == ErrorKind::AddrInUse => {
            if let Ok(mut stream) = UnixStream::connect(path) {
                stream.set_write_timeout(Some(Duration::from_secs(1)))?;
                if stream.write_all(format!("{SHOW}\n").as_bytes()).is_ok() {
                    return Ok(Claim::HandedOff);
                }
            }
            // Nobody answers: a crashed instance left the socket behind.
            std::fs::remove_file(path)?;
            bind(path).map(Claim::Primary)
        }
        Err(error) => Err(error),
    }
}

fn bind(path: &Path) -> std::io::Result<UnixListener> {
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Forwards each `show` request on a background thread. Other input is ignored.
pub fn serve(listener: UnixListener, show: UnboundedSender<()>) {
    std::thread::Builder::new()
        .name("me-single-instance".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                let mut line = String::new();
                // Cap the request so a stray client cannot grow the buffer.
                let mut reader = BufReader::new(stream.take(16));
                if reader.read_line(&mut line).is_ok()
                    && line.trim_end() == SHOW
                    && show.unbounded_send(()).is_err()
                {
                    break;
                }
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_channel::mpsc::unbounded;

    fn wait_for(receiver: &mut futures_channel::mpsc::UnboundedReceiver<()>) -> Option<()> {
        for _ in 0..200 {
            if let Ok(value) = receiver.try_recv() {
                return Some(value);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }

    #[test]
    fn second_launch_asks_the_first_to_show_itself() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("me.sock");
        let Claim::Primary(listener) = claim(&path).unwrap() else {
            panic!("first launch must be primary");
        };
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let (show, mut received) = unbounded();
        serve(listener, show);
        assert!(matches!(claim(&path).unwrap(), Claim::HandedOff));
        assert_eq!(wait_for(&mut received), Some(()));
    }

    #[test]
    fn stale_socket_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("me.sock");
        drop(UnixListener::bind(&path).unwrap());
        assert!(path.exists());
        assert!(matches!(claim(&path).unwrap(), Claim::Primary(_)));
    }

    #[test]
    fn unknown_commands_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("me.sock");
        let Claim::Primary(listener) = claim(&path).unwrap() else {
            panic!("first launch must be primary");
        };
        let (show, mut received) = unbounded();
        serve(listener, show);
        UnixStream::connect(&path)
            .unwrap()
            .write_all(b"open /etc/passwd\n")
            .unwrap();
        assert_eq!(wait_for(&mut received), None);
        UnixStream::connect(&path)
            .unwrap()
            .write_all(b"show\n")
            .unwrap();
        assert_eq!(wait_for(&mut received), Some(()));
    }

    #[test]
    fn socket_path_is_stable_short_and_per_vault() {
        let dev = socket_path(Path::new(
            "/Users/a/Library/Application Support/ME Dev/vault",
        ));
        let preview = socket_path(Path::new("/tmp/preview/vault"));
        assert_ne!(dev, preview);
        assert_eq!(
            dev,
            socket_path(Path::new(
                "/Users/a/Library/Application Support/ME Dev/vault"
            ))
        );
        assert!(dev.file_name().unwrap().len() < 30);
    }
}
