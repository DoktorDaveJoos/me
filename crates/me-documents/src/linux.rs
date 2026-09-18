//! Poppler + Tesseract adapters. All document bytes travel through pipes.
use crate::process::run;
use me_core::{DocumentPart, MAX_DOCUMENT_PAGES, MAX_DOCUMENT_TEXT};
use std::{process::Command, sync::atomic::AtomicBool};
fn command(name: &str) -> Result<Command, String> {
    // Desktop launch environments may have a very small PATH.
    let path = std::path::Path::new("/usr/bin").join(name);
    if !path.is_file() {
        return Err(format!(
            "Local processing requires {name}. Install poppler-utils, tesseract-ocr, tesseract-ocr-deu, tesseract-ocr-eng, and unrtf."
        ));
    }
    let mut command = Command::new(path);
    command.env("LC_ALL", "C");
    Ok(command)
}

fn output(
    command: &mut Command,
    bytes: &[u8],
    cancel: &AtomicBool,
    limit: usize,
) -> Result<zeroize::Zeroizing<Vec<u8>>, String> {
    let (status, out) = run(command, bytes, cancel, limit)?;
    if status != 0 {
        return Err("Local processing failed. Check the format, PDF password protection, and installed German and English OCR languages.".into());
    }
    Ok(out)
}
fn ocr(bytes: &[u8], cancel: &AtomicBool) -> Result<String, String> {
    let mut cmd = command("tesseract")?;
    let out = output(
        cmd.args(["stdin", "stdout", "-l", "deu+eng"]),
        bytes,
        cancel,
        MAX_DOCUMENT_TEXT,
    )?;
    String::from_utf8(out.to_vec()).map_err(|_| "Text recognition returned invalid text.".into())
}
#[cfg(test)]
pub(crate) fn extract(
    extension: &str,
    bytes: &[u8],
    cancel: &AtomicBool,
) -> Result<Vec<DocumentPart>, String> {
    extract_with_progress(extension, bytes, cancel, &mut |_, _| {})
}
pub(crate) fn extract_with_progress(
    extension: &str,
    bytes: &[u8],
    cancel: &AtomicBool,
    progress: &mut impl FnMut(u32, u32),
) -> Result<Vec<DocumentPart>, String> {
    if ["heic", "heif", "gif", "webp"].contains(&extension) {
        return Err(
            "Dieses Bildformat bitte unter Linux zuerst als JPEG, PNG oder PDF exportieren.".into(),
        );
    }
    if extension == "rtf" {
        if !bytes.starts_with(b"{\\rtf") {
            return Err("This isn't a valid RTF file.".into());
        }
        let mut cmd = command("unrtf")?;
        let out = output(
            cmd.args(["--text", "--nopict", "--quiet"]),
            bytes,
            cancel,
            MAX_DOCUMENT_TEXT,
        )?;
        let text =
            String::from_utf8(out.to_vec()).map_err(|_| "RTF-Text konnte nicht gelesen werden.")?;
        return Ok(vec![DocumentPart {
            text,
            page: None,
            section: None,
            method: "office".into(),
        }]);
    }
    let mut parts = Vec::new();
    if extension == "pdf" {
        if !bytes.starts_with(b"%PDF-") {
            return Err("This isn't a valid PDF.".into());
        }
        let mut cmd = command("pdftotext")?;
        let out = output(
            cmd.args(["-layout", "-enc", "UTF-8", "-", "-"]),
            bytes,
            cancel,
            MAX_DOCUMENT_TEXT,
        )?;
        let text =
            std::str::from_utf8(&out).map_err(|_| "PDF-Text konnte nicht gelesen werden.")?;
        let pages: Vec<_> = text
            .strip_suffix('\u{c}')
            .unwrap_or(text)
            .split('\u{c}')
            .collect();
        if pages.len() > MAX_DOCUMENT_PAGES as usize {
            return Err("PDFs dürfen höchstens 500 Seiten enthalten.".into());
        }
        let total = pages.len() as u32;
        progress(0, total);
        for (index, text) in pages.into_iter().enumerate() {
            let page = index as u32 + 1;
            if !text.trim().is_empty() {
                parts.push(DocumentPart {
                    text: text.into(),
                    page: Some(page),
                    section: None,
                    method: "pdf_text".into(),
                });
            }
            let mut render = command("pdftoppm")?;
            let number = page.to_string();
            let png = output(
                render.args([
                    "-f",
                    &number,
                    "-l",
                    &number,
                    "-singlefile",
                    "-scale-to",
                    "3200",
                    "-png",
                    "-",
                ]),
                bytes,
                cancel,
                40 * 1024 * 1024,
            )?;
            let recognized = ocr(&png, cancel)?;
            let key = |s: &str| {
                s.to_lowercase()
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>()
            };
            let existing = key(text);
            let additional = recognized
                .lines()
                .filter(|l| !key(l).is_empty() && !existing.contains(&key(l)))
                .collect::<Vec<_>>()
                .join("\n");
            if !additional.is_empty() {
                parts.push(DocumentPart {
                    text: additional,
                    page: Some(page),
                    section: None,
                    method: "ocr".into(),
                });
            }
            progress(page, total);
            if parts.iter().map(|p| p.text.len()).sum::<usize>() > MAX_DOCUMENT_TEXT {
                return Err("Extracted text exceeds 8 MiB.".into());
            }
        }
    } else {
        let text = ocr(bytes, cancel)?;
        for (i, text) in text.trim_end_matches('\u{c}').split('\u{c}').enumerate() {
            if i >= MAX_DOCUMENT_PAGES as usize {
                return Err("Bilddateien dürfen höchstens 500 Seiten enthalten.".into());
            }
            if !text.trim().is_empty() {
                parts.push(DocumentPart {
                    text: text.into(),
                    page: Some(i as u32 + 1),
                    section: None,
                    method: "ocr".into(),
                });
            }
        }
    }
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_files_are_rejected_before_external_tools() {
        let cancel = AtomicBool::new(false);
        assert!(
            extract("pdf", b"not a PDF", &cancel)
                .unwrap_err()
                .contains("valid PDF")
        );
        assert!(
            extract("rtf", b"not RTF", &cancel)
                .unwrap_err()
                .contains("valid RTF")
        );
    }
}
