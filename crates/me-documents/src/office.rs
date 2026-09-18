use me_core::{DocumentPart, MAX_DOCUMENT_TEXT};
use quick_xml::{Reader, events::Event};
use std::{
    io::{Cursor, Read},
    sync::atomic::{AtomicBool, Ordering},
};
use zeroize::Zeroizing;
const INVALID: &str = "This Office file is damaged, encrypted, or unsupported.";

pub(crate) fn extract(
    extension: &str,
    bytes: &[u8],
    cancel: &AtomicBool,
) -> Result<Vec<DocumentPart>, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| INVALID)?;
    if archive.len() > 4096 {
        return Err("This Office file has too many parts.".into());
    }
    let mut names = vec![
        if extension == "docx" {
            "word/document.xml"
        } else {
            "content.xml"
        }
        .to_string(),
    ];
    if extension == "docx" {
        let mut extra: Vec<_> = archive
            .file_names()
            .filter(|n| {
                n.starts_with("word/")
                    && !n[5..].contains('/')
                    && n.ends_with(".xml")
                    && (n.starts_with("word/header")
                        || n.starts_with("word/footer")
                        || *n == "word/footnotes.xml"
                        || *n == "word/endnotes.xml")
            })
            .map(str::to_string)
            .collect();
        extra.sort();
        names.extend(extra);
    }
    if names.len() > 100 {
        return Err("This Office file has too many text sections.".into());
    }
    let mut parts = Vec::new();
    let mut total = 0;
    let mut text_total = 0;
    for name in names {
        if cancel.load(Ordering::SeqCst) {
            return Err("File processing stopped.".into());
        }
        let file = archive.by_name(&name).map_err(|_| INVALID)?;
        if file.size() > 8 * 1024 * 1024 {
            return Err("This Office file exceeds the content size limit.".into());
        }
        let mut xml = Zeroizing::new(Vec::new());
        file.take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut xml)
            .map_err(|_| INVALID)?;
        total += xml.len();
        if xml.len() > 8 * 1024 * 1024 || total > 16 * 1024 * 1024 {
            return Err("This Office file exceeds the content size limit.".into());
        }
        let text = xml_text(&xml, extension == "docx", cancel)?;
        text_total += text.len();
        if text_total > MAX_DOCUMENT_TEXT {
            return Err("Extracted text exceeds 1 MiB.".into());
        }
        if !text.trim().is_empty() {
            let section = if name.contains("header") {
                "Header"
            } else if name.contains("footer") {
                "Footer"
            } else if name.contains("notes") {
                "Footnotes/endnotes"
            } else {
                "Document text"
            };
            parts.push(DocumentPart {
                text,
                page: None,
                section: Some(section.into()),
                method: "office".into(),
            });
        }
    }
    Ok(parts)
}

fn xml_text(xml: &[u8], docx: bool, cancel: &AtomicBool) -> Result<String, String> {
    let mut reader = Reader::from_reader(xml);
    let mut text = String::new();
    let mut depth = 0;
    let mut capture = 0;
    let mut skip = 0;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err("File processing stopped.".into());
        }
        match reader.read_event().map_err(|_| INVALID)? {
            Event::Start(e) => {
                depth += 1;
                if depth > 128 {
                    return Err(INVALID.into());
                }
                let local = e.local_name();
                let name = local.as_ref();
                if skip == 0
                    && (name == b"del" || name == b"tracked-changes" || name == b"annotation")
                {
                    skip = depth;
                }
                if skip == 0
                    && capture == 0
                    && ((docx && name == b"t") || (!docx && (name == b"p" || name == b"h")))
                {
                    capture = depth;
                }
            }
            Event::End(e) => {
                if skip == 0
                    && matches!(e.local_name().as_ref(), b"p" | b"h" | b"tr" | b"table-row")
                {
                    text.push('\n');
                }
                if capture == depth {
                    capture = 0;
                }
                if skip == depth {
                    skip = 0;
                }
                depth -= 1;
            }
            Event::Empty(e) if skip == 0 => match e.local_name().as_ref() {
                b"tab" => text.push('\t'),
                b"br" | b"line-break" => text.push('\n'),
                b"s" if !docx => {
                    let mut count = 1;
                    for attr in e.attributes() {
                        let attr = attr.map_err(|_| INVALID)?;
                        if attr.key.local_name().as_ref() == b"c" {
                            count = std::str::from_utf8(&attr.value)
                                .map_err(|_| INVALID)?
                                .parse::<usize>()
                                .map_err(|_| INVALID)?;
                        }
                    }
                    if count > 10000 {
                        return Err(INVALID.into());
                    }
                    text.extend(std::iter::repeat_n(' ', count));
                }
                _ => {}
            },
            Event::Text(e) if capture > 0 && skip == 0 => {
                text.push_str(&e.xml10_content().map_err(|_| INVALID)?)
            }
            Event::CData(e) if capture > 0 && skip == 0 => {
                text.push_str(&e.decode().map_err(|_| INVALID)?)
            }
            Event::GeneralRef(e) if capture > 0 && skip == 0 => {
                if let Some(ch) = e.resolve_char_ref().map_err(|_| INVALID)? {
                    text.push(ch);
                } else {
                    text.push_str(
                        quick_xml::escape::resolve_predefined_entity(
                            &e.decode().map_err(|_| INVALID)?,
                        )
                        .ok_or(INVALID)?,
                    );
                }
            }
            Event::DocType(_) => return Err(INVALID.into()),
            Event::Eof => break,
            _ => {}
        }
        if text.len() > MAX_DOCUMENT_TEXT {
            return Err("Extracted text exceeds 1 MiB.".into());
        }
    }
    if depth != 0 {
        return Err(INVALID.into());
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn archive(name: &str, xml: &str) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(xml.as_bytes()).unwrap();
        zip.finish().unwrap().into_inner()
    }
    #[test]
    fn docx_preserves_split_runs_entities_and_excludes_deleted_values() {
        let data = archive(
            "word/document.xml",
            r#"<w:document xmlns:w="urn:test"><w:body><w:p><w:r><w:t>SYNTHETIC M&#252;ller &amp; Co</w:t></w:r></w:p><w:p><w:r><w:t>Steuer-ID: 01234</w:t></w:r><w:r><w:t>567890</w:t></w:r><w:del><w:r><w:delText>OLD</w:delText></w:r></w:del></w:p></w:body></w:document>"#,
        );
        let parts = extract("docx", &data, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            parts[0].text,
            "SYNTHETIC Müller & Co\nSteuer-ID: 01234567890\n"
        );
        assert!(parts[0].page.is_none());
    }
    #[test]
    fn odt_retains_paragraph_spaces_and_rejects_entities() {
        let data = archive(
            "content.xml",
            r#"<office:document xmlns:office="urn:a" xmlns:text="urn:b"><text:p>Geburtsdatum:<text:s text:c="2"/>01.02.1990</text:p><text:p>Name: Beispiel</text:p></office:document>"#,
        );
        let parts = extract("odt", &data, &AtomicBool::new(false)).unwrap();
        assert!(parts[0].text.contains("Geburtsdatum:  01.02.1990\nName:"));
        let bad = archive(
            "content.xml",
            r#"<!DOCTYPE x [<!ENTITY secret SYSTEM "file:///etc/passwd">]><p>&secret;</p>"#,
        );
        assert!(extract("odt", &bad, &AtomicBool::new(false)).is_err());
    }
    #[test]
    fn archive_limits_and_cancel_are_not_partial_successes() {
        let huge = archive(
            "content.xml",
            &format!("<p>{}</p>", "x".repeat(8 * 1024 * 1024)),
        );
        assert!(extract("odt", &huge, &AtomicBool::new(false)).is_err());
        let valid = archive("content.xml", "<p>synthetic</p>");
        assert!(extract("odt", &valid, &AtomicBool::new(true)).is_err());
        assert!(extract("docx", &valid, &AtomicBool::new(false)).is_err());
    }
}
