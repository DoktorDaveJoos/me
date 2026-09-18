//! Decode MIME locally; never fetch remote images/links or execute HTML.
use mail_parser::{Address, Message, MessageParser, MimeHeaders, PartType};
use me_core::{DocumentPart, MAX_DOCUMENT_TEXT};
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) fn extract(
    bytes: &[u8],
    cancel: &AtomicBool,
    progress: &mut impl FnMut(u32, u32),
) -> Result<Vec<DocumentPart>, String> {
    let message = MessageParser::default()
        .parse(bytes)
        .ok_or("This email could not be decoded. Export it as EML or PDF again.")?;
    let mut parts = Vec::new();
    read_message(&message, "Email", 0, &mut parts, cancel, progress)?;
    Ok(parts)
}
fn addresses(value: Option<&Address<'_>>) -> String {
    value
        .map(|a| {
            a.iter()
                .map(|a| match (&a.name, &a.address) {
                    (Some(name), Some(address)) => format!("{name} <{address}>"),
                    (None, Some(address)) => address.to_string(),
                    (Some(name), None) => name.to_string(),
                    _ => String::new(),
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}
fn push(
    parts: &mut Vec<DocumentPart>,
    text: String,
    section: String,
    method: &str,
) -> Result<(), String> {
    if parts.len() >= me_core::MAX_DOCUMENT_PARTS
        || parts.iter().map(|p| p.text.len()).sum::<usize>() + text.len() > MAX_DOCUMENT_TEXT
    {
        return Err("Decoded email exceeds the text limit.".into());
    }
    parts.push(DocumentPart {
        text,
        page: None,
        section: Some({
            let mut end = section.len().min(190);
            while !section.is_char_boundary(end) {
                end -= 1;
            }
            section[..end].to_owned()
        }),
        method: method.into(),
    });
    Ok(())
}
fn read_message(
    message: &Message<'_>,
    location: &str,
    depth: usize,
    parts: &mut Vec<DocumentPart>,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(u32, u32),
) -> Result<(), String> {
    if depth > 5 || message.parts.len() > 512 || message.attachment_count() > 64 {
        return Err(
            "Email exceeds the attachment or nesting limit. Import attachments separately.".into(),
        );
    }
    if cancel.load(Ordering::SeqCst) {
        return Err("Email processing stopped.".into());
    }
    if message.parts.iter().any(|p| p.is_encoding_problem) {
        return Err(
            "Some email content could not be decoded reliably. Export a new EML or PDF copy."
                .into(),
        );
    }
    if message.from().is_none() && message.subject().is_none() {
        return Err("This file has no recognizable email headers.".into());
    }
    let date = message
        .headers_raw()
        .find(|(name, _)| name.eq_ignore_ascii_case("date"))
        .map(|(_, value)| value.trim())
        .unwrap_or("");
    push(
        parts,
        format!(
            "From: {}\nTo: {}\nCc: {}\nDate: {}\nSubject: {}",
            addresses(message.from()),
            addresses(message.to()),
            addresses(message.cc()),
            date,
            message.subject().unwrap_or("")
        ),
        format!("{location} / headers"),
        "text",
    )?;
    for index in 0..message.text_body_count() {
        if let Some(body) = message.body_text(index)
            && !body.trim().is_empty()
        {
            push(
                parts,
                body.into_owned(),
                format!("{location} / body {}", index + 1),
                "text",
            )?;
        }
    }
    // Alternatives sometimes contain additional or contradictory information.
    // Keep distinct HTML text rather than silently trusting the plain alternative.
    let plain = (0..message.text_body_count())
        .filter_map(|i| message.body_text(i))
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>();
    for index in 0..message.html_body_count() {
        if let Some(html) = message.body_html(index) {
            let text = mail_parser::decoders::html::html_to_text(&html);
            let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
            if !normalized.is_empty() && !plain.contains(&normalized) {
                push(
                    parts,
                    text,
                    format!("{location} / alternative HTML body {}", index + 1),
                    "text",
                )?;
            }
        }
    }
    for (index, attachment) in message.attachments().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            return Err("Email processing stopped.".into());
        }
        let name = attachment.attachment_name().unwrap_or("unnamed attachment");
        let section = format!(
            "{location} / attachment {}: {}",
            index + 1,
            name.chars().take(70).collect::<String>()
        );
        if let PartType::Message(nested) = &attachment.body {
            read_message(nested, &section, depth + 1, parts, cancel, progress)?;
            continue;
        }
        let extension = std::path::Path::new(name)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let mime_extension =
            attachment
                .content_type()
                .and_then(|t| match (t.ctype(), t.subtype()) {
                    ("application", Some("pdf")) => Some("pdf"),
                    ("image", Some("jpeg")) => Some("jpg"),
                    ("image", Some("png")) => Some("png"),
                    ("text", Some("plain")) => Some("txt"),
                    _ => None,
                });
        let extension = if extension.is_empty() {
            mime_extension.unwrap_or("")
        } else {
            &extension
        };
        if !me_core::processable_document(extension) || extension == "eml" {
            return Err(format!(
                "Attachment {} could not be read with a supported parser. Import it separately as PDF, image or text; the original email is saved.",
                index + 1
            ));
        }
        let extracted =
            super::extract_with_progress(extension, attachment.contents(), cancel, progress)
                .map_err(|e| format!("Attachment {}: {e}", index + 1))?;
        for part in extracted {
            let label = part
                .page
                .map(|p| format!("page {p}"))
                .or(part.section)
                .unwrap_or_else(|| "text".into());
            push(
                parts,
                part.text,
                format!("{section} / {label}"),
                &part.method,
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn read(text: &str) -> Result<Vec<DocumentPart>, String> {
        extract(text.as_bytes(), &AtomicBool::new(false), &mut |_, _| {})
    }
    #[test]
    fn decodes_subject_addresses_and_quoted_printable_without_changing_values() {
        let parts=read("From: =?UTF-8?Q?Erika_Beispiel?= <erika@example.test>\r\nTo: kasse@example.test\r\nSubject: =?UTF-8?Q?Beitr=C3=A4ge?=\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nMitglied: 000123\r\nBeitrag: 123,45 =E2=82=AC").unwrap();
        assert!(parts[0].text.contains("Beiträge"));
        assert!(parts[0].text.contains("Erika Beispiel"));
        assert!(parts[1].text.contains("000123"));
        assert!(parts[1].text.contains("123,45 €"));
    }
    #[test]
    fn html_only_email_is_read_without_executing_or_fetching_content() {
        let parts=read("From: insurer@example.test\nSubject: Notice\nContent-Type: text/html; charset=utf-8\n\n<html><head><script>steal()</script></head><body><p>Policy 00042</p><img src='https://example.test/track'></body></html>").unwrap();
        assert!(parts.iter().any(|p| p.text.contains("Policy 00042")));
        assert!(!parts.iter().any(|p| p.text.contains("steal()")));
    }
    #[test]
    fn preserves_information_unique_to_an_html_alternative() {
        let parts = read("From: a@example.test\nSubject: Notice\nContent-Type: multipart/alternative; boundary=x\n\n--x\nContent-Type: text/plain\n\nYour notice\n--x\nContent-Type: text/html\n\n<p>Your notice</p><p>Policy: 00042</p>\n--x--").unwrap();
        assert!(parts.iter().any(|p| p.text.contains("Policy: 00042")));
    }
    #[test]
    fn attachments_are_evidence_not_silently_skipped() {
        let email = "From: a@example.test\nSubject: Notice\nContent-Type: multipart/mixed; boundary=x\n\n--x\nContent-Type: text/plain\n\nSee attachment\n--x\nContent-Type: text/plain\nContent-Disposition: attachment; filename=policy.txt\nContent-Transfer-Encoding: base64\n\nUG9saWN5OiAwMDA0Mg==\n--x--";
        let parts = read(email).unwrap();
        assert!(parts.iter().any(|p| p.text.contains("Policy: 00042")
            && p.section.as_deref().unwrap().contains("attachment")));
        assert!(
            read(
                &email
                    .replace(
                        "text/plain\nContent-Disposition",
                        "application/octet-stream\nContent-Disposition"
                    )
                    .replace("policy.txt", "policy.xyz")
            )
            .is_err()
        );
    }
}
