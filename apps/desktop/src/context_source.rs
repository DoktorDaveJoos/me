//! Explicit window previews and reviewed autofill. No background monitoring.
use base64::{Engine, engine::general_purpose::STANDARD};
use gpui::RenderImage;
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;
#[derive(Clone)]
pub struct Source {
    pub identity: serde_json::Value,
    pub name: String,
    pub title: String,
    pub preview: Option<Arc<RenderImage>>,
}
pub struct Sources {
    pub windows: Vec<Source>,
    pub permission: bool,
    pub more: bool,
}
#[cfg(target_os = "macos")]
fn helper() -> std::path::PathBuf {
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join("me-context")));
    bundled
        .filter(|p| p.is_file())
        .unwrap_or_else(|| std::path::PathBuf::from(env!("ME_CONTEXT_HELPER")))
}
#[cfg(not(target_os = "macos"))]
fn helper() -> std::path::PathBuf {
    std::path::PathBuf::from("/nonexistent/me-context")
}
fn run(
    args: &[&str],
    input: Option<Zeroizing<Vec<u8>>>,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<Zeroizing<Vec<u8>>, &'static str> {
    if !cfg!(target_os = "macos") {
        return Err(
            "Window previews and filling are available on macOS. Choose a type or paste on this device.",
        );
    }
    if cancel.is_some_and(|c| c.load(std::sync::atomic::Ordering::SeqCst)) {
        return Err("Filling cancelled. The login remains saved.");
    }
    let mut child = Command::new(helper())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Window access is unavailable. Choose a type instead.")?;
    let stdin = child.stdin.take();
    let writer = std::thread::spawn(move || {
        if let (Some(mut stdin), Some(input)) = (stdin, input) {
            let _ = stdin.write_all(&input);
        }
    });
    let stdout = child
        .stdout
        .take()
        .ok_or("Couldn't read the selected window.")?;
    let limit = if args.first() == Some(&"windows") {
        8 * 1024 * 1024
    } else {
        256 * 1024
    };
    let reader = std::thread::spawn(move || {
        let mut bytes = Zeroizing::new(Vec::new());
        stdout
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None)
                if Instant::now() < deadline
                    && !cancel.is_some_and(|c| c.load(std::sync::atomic::Ordering::SeqCst)) =>
            {
                std::thread::sleep(Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let _ = writer.join();
    let bytes = reader
        .join()
        .ok()
        .and_then(Result::ok)
        .ok_or("Couldn't read the selected window.")?;
    if !status.is_some_and(|s| s.success()) || bytes.len() > limit as usize {
        return Err("Window access timed out or could not start. Try again.");
    }
    Ok(bytes)
}
#[derive(Clone, Copy)]
pub enum PermissionKind {
    ScreenRecording,
    Accessibility,
}
#[derive(serde::Deserialize)]
pub struct Permissions {
    pub screen_recording: bool,
    pub accessibility: bool,
}
pub fn permissions(request: Option<PermissionKind>) -> Result<Permissions, &'static str> {
    let args: &[&str] = match request {
        None => &["permissions"],
        Some(PermissionKind::ScreenRecording) => &["permissions", "screen"],
        Some(PermissionKind::Accessibility) => &["permissions", "accessibility"],
    };
    let bytes = run(args, None, None)?;
    serde_json::from_slice(&bytes).map_err(|_| "Couldn't check permissions. Try again.")
}
pub fn sources(request_permission: bool) -> Result<Sources, &'static str> {
    let bytes = run(
        if request_permission {
            &["windows", "request"]
        } else {
            &["windows"]
        },
        None,
        None,
    )?;
    let mut value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "Couldn't load window previews.")?;
    if value["error"] == "version" {
        return Err("Window previews require macOS 14 or later. You can still choose a type.");
    }
    let permission = value["permission"]
        .as_bool()
        .ok_or("Couldn't load window previews.")?;
    let more = value["more"].as_bool().unwrap_or(false);
    let windows = value["windows"]
        .as_array_mut()
        .ok_or("Couldn't load window previews.")?
        .iter_mut()
        .take(12)
        .map(|v| {
            let preview = v
                .as_object_mut()
                .and_then(|o| o.remove("preview"))
                .and_then(|v| {
                    let encoded = Zeroizing::new(v.as_str()?.to_owned());
                    let bytes = Zeroizing::new(STANDARD.decode(encoded.as_bytes()).ok()?);
                    decode_preview(&bytes)
                });
            Source {
                name: v["name"].as_str().unwrap_or("Application").into(),
                title: v["title"].as_str().unwrap_or("Window").into(),
                identity: v.take(),
                preview,
            }
        })
        .collect();
    Ok(Sources {
        windows,
        permission,
        more,
    })
}
fn decode_preview(bytes: &[u8]) -> Option<Arc<RenderImage>> {
    if bytes.len() > 1_048_576 {
        return None;
    }
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Jpeg);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(1024);
    limits.max_image_height = Some(1024);
    limits.max_alloc = Some(8 * 1024 * 1024);
    reader.limits(limits);
    let mut pixels = reader.decode().ok()?.to_rgba8();
    for pixel in pixels.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])))
}
#[derive(Debug, PartialEq)]
pub enum CaptureError {
    Accessibility,
    Unavailable(&'static str),
}
impl From<&'static str> for CaptureError {
    fn from(message: &'static str) -> Self {
        Self::Unavailable(message)
    }
}
fn captured_window(bytes: &[u8]) -> Result<super::credential_capture::Capture, CaptureError> {
    // Deserialize only the status here, without making another copy of captured values.
    #[derive(serde::Deserialize)]
    struct Status {
        error: Option<String>,
    }
    let status: Status = serde_json::from_slice(bytes)
        .map_err(|_| CaptureError::Unavailable("Couldn't read this window."))?;
    if status.error.as_deref() == Some("permission") {
        return Err(CaptureError::Accessibility);
    }
    super::credential_capture::Capture::from_window(bytes).map_err(Into::into)
}
pub fn capture(
    source: &Source,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<super::credential_capture::Capture, CaptureError> {
    let input = Zeroizing::new(
        serde_json::to_vec(&source.identity).map_err(|_| "Invalid window selection.")?,
    );
    let bytes = run(&["capture"], Some(input), Some(cancel))?;
    captured_window(&bytes)
}
pub fn request_accessibility() -> Result<(), &'static str> {
    let bytes = run(&["accessibility", "request"], None, None)?;
    let status: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| "Couldn't request Accessibility access. Open System Settings to enable ME.")?;
    // false is expected while the asynchronous system prompt is displayed.
    status["accessibility"]
        .as_bool()
        .map(|_| ())
        .ok_or("Couldn't request Accessibility access. Open System Settings to enable ME.")
}
pub fn fill(
    target: &str,
    fields: &[(String, Zeroizing<String>)],
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<(), &'static str> {
    // Serialize directly into a zeroizing buffer; no secrets in argv, files or logs.
    let mut body = Zeroizing::new(Vec::new());
    let target: serde_json::Value =
        serde_json::from_str(target).map_err(|_| "The selected page is unavailable.")?;
    let field = |key: &str| {
        fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .unwrap_or("")
    };
    if field("username").is_empty() || field("password").is_empty() {
        return Err("Add an email and password before filling the page.");
    }
    body.extend_from_slice(b"{\"target\":");
    serde_json::to_writer(&mut *body, &target).map_err(|_| "Couldn't prepare autofill.")?;
    for key in ["username", "password", "full_name"] {
        body.extend_from_slice(format!(",\"{key}\":").as_bytes());
        serde_json::to_writer(&mut *body, field(key)).map_err(|_| "Couldn't prepare autofill.")?;
    }
    body.push(b'}');
    if body.len() > 65536 {
        return Err("This draft is too large to fill. Copy its fields individually.");
    }
    let bytes = run(&["fill"], Some(body), Some(cancel)).map_err(|_| "The login is saved, but filling was not confirmed. Check the page before trying again.")?;
    let result: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| "Couldn't confirm autofill. Check the page before trying again.")?;
    if result["filled"] == true {
        return Ok(());
    }
    Err(match result["error"].as_str() {
        Some("changed") => {
            "The page or form changed. The login is saved; reopen New item to select the current page."
        }
        Some("occupied") => {
            "The form already contains different values. The login is saved; check the page before filling it."
        }
        Some("permission") => "Allow ME. in Accessibility to fill this page. The login is saved.",
        Some("required") => {
            "A required name is missing. The login is saved; complete the form on the website."
        }
        Some("partial") => {
            "Only part of the form could be filled. The login is saved; check the website."
        }
        _ => {
            "This page doesn't support filling through ME. The login is saved; copy its fields to the page."
        }
    })
}

/// An edited website must never silently keep an earlier autofill destination.
pub fn fill_matches_website(target: &str, website: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(target) else {
        return false;
    };
    let Some(page) = value["page"]
        .as_str()
        .and_then(super::credential_capture::origin)
    else {
        return false;
    };
    super::credential_capture::origin(website).as_deref() == Some(page.as_str())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accessibility_denial_is_recoverable_without_losing_the_window() {
        assert!(matches!(
            captured_window(br#"{"error":"permission"}"#),
            Err(CaptureError::Accessibility)
        ));
        assert!(matches!(
            captured_window(br#"{"error":"window_identity"}"#),
            Err(CaptureError::Unavailable(_))
        ));
        assert!(matches!(
            captured_window(b"not json"),
            Err(CaptureError::Unavailable(_))
        ));
        let capture = captured_window(
            br#"{"source":"Synthetic","nodes":[{"label":"Email"}],"url":"https://example.test"}"#,
        )
        .unwrap();
        assert_eq!(capture.website, "https://example.test");
    }
    #[test]
    fn changed_origins_cannot_reuse_a_fill_target() {
        let target = r#"{"page":"https://forge.laravel.com/register"}"#;
        assert!(fill_matches_website(target, "https://forge.laravel.com"));
        assert!(!fill_matches_website(target, "http://forge.laravel.com"));
        assert!(!fill_matches_website(
            target,
            "https://forge.laravel.com.example.test"
        ));
        assert!(!fill_matches_website(target, "https://other.example.test"));
        assert!(!fill_matches_website("{}", "https://forge.laravel.com"));
    }
}
