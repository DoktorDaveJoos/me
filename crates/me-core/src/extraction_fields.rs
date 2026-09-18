//! Open-ended document facts. Keys and display context are scoped by trusted code,
//! so monthly pay and different accounts never become one timeless profile value.
use crate::vault::hash;
use crate::{Error, ExtractedFact, Result, STANDARD_FIELDS};
use rusqlite::{Transaction, params};
use serde_json::json;

pub fn document_field_label(key: &str) -> Option<&str> {
    let label = key.strip_prefix("document.")?;
    (!label.trim().is_empty()
        && label == label.trim()
        && label.len() <= 180
        && !label.chars().any(char::is_control))
    .then_some(label)
}

pub(crate) fn ensure_extraction_property(
    tx: &Transaction<'_>,
    source: &str,
    fact: &ExtractedFact,
) -> Result<String> {
    if STANDARD_FIELDS.iter().any(|f| f.key == fact.property) {
        return Ok(fact.property.clone());
    }
    let label =
        document_field_label(&fact.property).ok_or(Error::Validation("Invalid document field."))?;
    if fact.context_quote.len() > 1000 {
        return Err(Error::Validation("Invalid document context."));
    }
    let context = fact
        .context_quote
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let subject = fact
        .subject_quote
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    // Without a period, preserve each source segment instead of guessing scope.
    let scope = if context.is_empty() {
        fact.segment_id.as_str()
    } else {
        &context
    };
    let key = format!(
        "document.{}",
        hash(
            json!([source, label.to_lowercase(), subject, scope])
                .to_string()
                .as_bytes()
        )
    );
    let title: String = tx.query_row("SELECT title FROM source WHERE id=?", [source], |r| {
        r.get(0)
    })?;
    let display = if context.is_empty() {
        format!("{label} · {title}")
    } else {
        format!("{label} · {context} · {title}")
    };
    let rules = json!({"source_id":source,"extracted_property":fact.property,"context_quote":fact.context_quote,"subject_quote":fact.subject_quote});
    tx.execute(
        "INSERT OR IGNORE INTO property_definition VALUES(?,?,'text','many','confirm',?,1)",
        params![key, display, rules.to_string()],
    )?;
    Ok(key)
}
