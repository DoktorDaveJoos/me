//! Local extraction only. No provider, network client, or document logging.
mod email;
#[cfg(any(target_os = "linux", test))]
mod linux;
mod office;
mod process;
use me_core::{DocumentPart, MAX_DOCUMENT_TEXT};
use std::sync::atomic::{AtomicBool, Ordering};

pub fn extract(
    extension: &str,
    bytes: &[u8],
    cancel: &AtomicBool,
) -> Result<Vec<DocumentPart>, String> {
    extract_with_progress(extension, bytes, cancel, &mut |_, _| {})
}

pub fn extract_with_progress(
    extension: &str,
    bytes: &[u8],
    cancel: &AtomicBool,
    progress: &mut impl FnMut(u32, u32),
) -> Result<Vec<DocumentPart>, String> {
    if cancel.load(Ordering::SeqCst) {
        return Err("File processing stopped.".into());
    }
    if bytes.len() > 64 * 1024 * 1024 {
        return Err("Documents must be no larger than 64 MiB.".into());
    }
    let extension = extension.to_ascii_lowercase();
    let parts = match extension.as_str() {
        "txt" | "md" | "csv" | "tsv" | "log" => {
            if bytes.len() > 1024 * 1024 {
                return Err("Text files must be no larger than 1 MiB.".into());
            }
            let text = std::str::from_utf8(bytes)
                .map_err(|_| "Save the text file with UTF-8 encoding.")?
                .trim_start_matches('\u{feff}')
                .to_string();
            vec![DocumentPart {
                text,
                page: None,
                section: None,
                method: "text".into(),
            }]
        }
        "eml" => email::extract(bytes, cancel, progress)?,
        "docx" | "odt" => office::extract(&extension, bytes, cancel)?,
        "pdf" | "jpg" | "jpeg" | "png" | "heic" | "heif" | "webp" | "tif" | "tiff" | "bmp"
        | "gif" | "rtf" => native(&extension, bytes, cancel, progress)?,
        _ => {
            return Err("Import a PDF, DOCX, ODT, RTF, or text version of this file.".into());
        }
    };
    if parts.is_empty() || parts.iter().all(|p| p.text.trim().is_empty()) {
        return Err(
            "No readable text found. Check the original's resolution and orientation.".into(),
        );
    }
    if parts.len() > me_core::MAX_DOCUMENT_PARTS
        || parts.iter().map(|p| p.text.len()).sum::<usize>() > MAX_DOCUMENT_TEXT
    {
        return Err("Extracted text exceeds 8 MiB.".into());
    }
    if parts.iter().any(|p| p.text.contains('\0')) {
        return Err("The file contains invalid text.".into());
    }
    if cancel.load(Ordering::SeqCst) {
        return Err("File processing stopped.".into());
    }
    Ok(parts)
}

#[cfg(target_os = "macos")]
fn native(
    extension: &str,
    bytes: &[u8],
    cancel: &AtomicBool,
    progress: &mut impl FnMut(u32, u32),
) -> Result<Vec<DocumentPart>, String> {
    use std::{io::Write, os::unix::fs::PermissionsExt, process::Command};
    // Embedded helper makes both `cargo run` and the app bundle self-contained.
    // This private temporary executable contains code only, never document data.
    let dir = tempfile::tempdir().map_err(|_| "Couldn't prepare text recognition.")?;
    let path = dir.path().join("me-document-extract");
    let mut file =
        std::fs::File::create(&path).map_err(|_| "Couldn't prepare text recognition.")?;
    file.write_all(include_bytes!(concat!(
        env!("OUT_DIR"),
        "/me-document-extract"
    )))
    .map_err(|_| "Couldn't prepare text recognition.")?;
    file.set_permissions(std::fs::Permissions::from_mode(0o700))
        .map_err(|_| "Couldn't prepare text recognition.")?;
    drop(file);
    let (status, out) = process::run_with_progress(
        Command::new(path).arg(extension),
        bytes,
        cancel,
        MAX_DOCUMENT_TEXT * 6 + 65536,
        progress,
    )?;
    match status {
        0 => serde_json::from_slice(&out)
            .map_err(|_| "Text recognition returned an invalid response.".into()),
        3 => Err("This PDF is password protected. Import an unlocked copy.".into()),
        4 => Err("The file exceeds 500 pages or the text/image size limit.".into()),
        5 => Err("No readable text found. Check the original's resolution and orientation.".into()),
        6 => Err("Text recognition failed. Try a clearer PDF or image.".into()),
        _ => Err("The file is damaged or doesn't match its format.".into()),
    }
}

#[cfg(target_os = "linux")]
fn native(
    extension: &str,
    bytes: &[u8],
    cancel: &AtomicBool,
    progress: &mut impl FnMut(u32, u32),
) -> Result<Vec<DocumentPart>, String> {
    linux::extract_with_progress(extension, bytes, cancel, progress)
}
