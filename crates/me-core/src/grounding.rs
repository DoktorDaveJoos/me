//! Deterministic evidence repair. Never change letters/digits, guess a value,
//! search another source, or accept model-written quotations as source text.
use crate::{ExtractedFact, ExtractionInput, ExtractionOutput, normalize_fact};

#[derive(Clone, Debug)]
pub struct RejectedFact {
    pub fact: ExtractedFact,
    pub code: &'static str,
}
pub struct GroundedExtraction {
    pub output: ExtractionOutput,
    pub rejected: Vec<RejectedFact>,
    pub repaired: usize,
}

// Map normalized characters back to exact UTF-8 slices. Whitespace is collapsed;
// between digits only, whitespace may be removed (grouped identifiers).
fn folded(text: &str, digits: bool) -> (String, Vec<(usize, usize)>) {
    let chars: Vec<_> = text.char_indices().collect();
    let mut result = String::new();
    let mut positions: Vec<(usize, usize)> = Vec::new();
    for (index, &(start, ch)) in chars.iter().enumerate() {
        let end = start + ch.len_utf8();
        if ch.is_whitespace() {
            let previous = chars[..index]
                .iter()
                .rev()
                .find(|(_, c)| !c.is_whitespace())
                .map(|(_, c)| *c);
            let next = chars[index + 1..]
                .iter()
                .find(|(_, c)| !c.is_whitespace())
                .map(|(_, c)| *c);
            if digits
                && previous.is_some_and(|c| c.is_ascii_digit())
                && next.is_some_and(|c| c.is_ascii_digit())
            {
                continue;
            }
            if result.ends_with(' ') {
                if let Some(last) = positions.last_mut() {
                    *last = (last.0, end);
                }
                continue;
            }
            result.push(' ');
            positions.push((start, end));
        } else {
            result.push(ch);
            positions.extend(std::iter::repeat_n((start, end), ch.len_utf8()));
        }
    }
    (result, positions)
}
fn whole_span(source: &str, start: usize, end: usize) -> bool {
    let first = source[start..end].chars().next();
    let last = source[start..end].chars().next_back();
    let before = source[..start].chars().next_back();
    let after = source[end..].chars().next();
    if first.is_some_and(char::is_alphanumeric) && before.is_some_and(char::is_alphanumeric)
        || last.is_some_and(char::is_alphanumeric) && after.is_some_and(char::is_alphanumeric)
    {
        return false;
    }
    if first.is_some_and(|c| c.is_ascii_digit()) {
        if matches!(before, Some('-' | '+' | '−')) {
            return false;
        }
        if matches!(before, Some('.' | ',' | '/'))
            && source[..start]
                .chars()
                .rev()
                .nth(1)
                .is_some_and(|c| c.is_ascii_digit())
        {
            return false;
        }
    }
    !(last.is_some_and(|c| c.is_ascii_digit())
        && matches!(after, Some('.' | ',' | '/'))
        && source[end..]
            .chars()
            .nth(1)
            .is_some_and(|c| c.is_ascii_digit()))
}
fn quote_span<'a>(source: &'a str, quote: &str, digits: bool) -> Option<&'a str> {
    if quote.trim().is_empty() {
        return None;
    }
    if let Some((start, _)) = source
        .match_indices(quote)
        .find(|(start, _)| whole_span(source, *start, *start + quote.len()))
    {
        return Some(&source[start..start + quote.len()]);
    }
    let (text, positions) = folded(source, digits);
    let (quote, _) = folded(quote, digits);
    let quote = quote.trim();
    for (start, _) in text.match_indices(quote) {
        let a = positions[start].0;
        let b = positions[start + quote.len() - 1].1;
        if whole_span(source, a, b) {
            return Some(&source[a..b]);
        }
    }
    None
}
pub(crate) fn contains_whole_value(source: &str, value: &str) -> bool {
    !value.is_empty()
        && source
            .match_indices(value)
            .any(|(start, _)| whole_span(source, start, start + value.len()))
}

fn raw_value<'a>(property: &str, value: &str, quote: &'a str) -> Option<&'a str> {
    let expected = normalize_fact(property, value).ok()?;
    if crate::document_field_label(property).is_some() {
        // Raw amounts, addresses, IBANs and words must be present, not synthesized.
        return quote_span(quote, value.trim(), false);
    }
    // Enumerate bounded original spans. This also resolves ISO vs German dates
    // while retaining the exact original spelling in the proposal quotation.
    let chars: Vec<_> = quote.char_indices().collect();
    for (i, &(start, first)) in chars.iter().enumerate() {
        if !first.is_ascii_digit() || (i > 0 && chars[i - 1].1.is_alphanumeric()) {
            continue;
        }
        for &(offset, last) in chars[i..].iter().take(40) {
            if !(last.is_ascii_alphanumeric() || last.is_whitespace() || last == '.' || last == '-')
            {
                break;
            }
            let end = offset + last.len_utf8();
            if last.is_whitespace()
                || quote[end..]
                    .chars()
                    .next()
                    .is_some_and(char::is_alphanumeric)
            {
                continue;
            }
            if normalize_fact(property, &quote[start..end]).ok().as_ref() == Some(&expected) {
                return Some(&quote[start..end]);
            }
        }
    }
    None
}

pub fn ground_extraction(input: &ExtractionInput, output: ExtractionOutput) -> GroundedExtraction {
    let mut result = GroundedExtraction {
        output: ExtractionOutput { facts: Vec::new() },
        rejected: Vec::new(),
        repaired: 0,
    };
    for mut fact in output.facts {
        let checked = (|| {
            if fact.quote.is_empty() || fact.quote.len() > 4000 || fact.subject_quote.len() > 1000 {
                return Err("invalid_quote_size");
            }
            let segment = input
                .segments
                .iter()
                .find(|s| s.segment_id == fact.segment_id)
                .ok_or("unknown_segment")?;
            let quote =
                quote_span(&segment.text, &fact.quote, true).ok_or("quote_not_in_segment")?;
            let value =
                raw_value(&fact.property, &fact.value, quote).ok_or("value_not_in_quote")?;
            let subject = if fact.subject_quote.trim().is_empty() {
                ""
            } else {
                input
                    .segments
                    .iter()
                    .find_map(|s| quote_span(&s.text, &fact.subject_quote, false))
                    .ok_or("subject_not_in_source")?
            };
            let context = if fact.context_quote.is_empty() {
                String::new()
            } else {
                if fact.context_quote.len() > 1000 {
                    return Err("invalid_context_size");
                }
                input
                    .segments
                    .iter()
                    .find_map(|s| quote_span(&s.text, &fact.context_quote, false))
                    .ok_or("context_not_in_source")?
                    .to_owned()
            };
            Ok((
                quote.to_owned(),
                value.to_owned(),
                subject.to_owned(),
                context,
            ))
        })();
        match checked {
            Ok((quote, value, subject, context)) => {
                result.repaired += usize::from(
                    quote != fact.quote || value != fact.value || subject != fact.subject_quote,
                );
                fact.quote = quote;
                fact.value = value;
                fact.subject_quote = subject;
                fact.context_quote = context;
                if fact.subject_quote.is_empty() {
                    // The value, quotation and context passed source checks.
                    // Ownership is a user decision, never a reason to drop it.
                    if !result
                        .rejected
                        .iter()
                        .any(|old| same_fact(&old.fact, &fact))
                    {
                        result.rejected.push(RejectedFact {
                            fact,
                            code: "subject_unknown",
                        });
                    }
                } else if !result.output.facts.iter().any(|old| same_fact(old, &fact)) {
                    result.output.facts.push(fact);
                }
            }
            Err(code) => result.rejected.push(RejectedFact { fact, code }),
        }
    }
    result
}

pub fn same_fact(a: &ExtractedFact, b: &ExtractedFact) -> bool {
    a.property == b.property
        && (crate::document_field_label(&a.property).is_none()
            || (a
                .context_quote
                .split_whitespace()
                .eq(b.context_quote.split_whitespace())
                && (!a.context_quote.is_empty() || a.segment_id == b.segment_id)))
        && match (
            normalize_fact(&a.property, &a.value),
            normalize_fact(&b.property, &b.value),
        ) {
            (Ok(a), Ok(b)) => a == b,
            _ => a.value == b.value,
        }
        && a.subject_quote
            .split_whitespace()
            .eq(b.subject_quote.split_whitespace())
}

/// Enrich an otherwise identical source field with evidenced context. Never
/// merge two explicit periods or different columns that merely share an amount.
pub fn merge_extracted_fact(facts: &mut Vec<ExtractedFact>, fact: ExtractedFact) {
    if facts.iter().any(|old| same_fact(old, &fact)) {
        return;
    }
    if crate::document_field_label(&fact.property).is_some() {
        for old in facts.iter_mut() {
            if old.property == fact.property
                && old.segment_id == fact.segment_id
                && normalize_fact(&old.property, &old.value).ok()
                    == normalize_fact(&fact.property, &fact.value).ok()
                && old
                    .subject_quote
                    .split_whitespace()
                    .eq(fact.subject_quote.split_whitespace())
            {
                if old.context_quote.is_empty() && !fact.context_quote.is_empty() {
                    *old = fact;
                    return;
                }
                if !old.context_quote.is_empty() && fact.context_quote.is_empty() {
                    return;
                }
            }
        }
    }
    facts.push(fact);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input() -> ExtractionInput {
        ExtractionInput {
            run_id: "run".into(),
            source_id: "source".into(),
            title: "Synthetic".into(),
            segments: vec![crate::SourceText {
                segment_id: "s1".into(),
                ordinal: 1,
                text: "Erika\nBeispiel\nSteuer-ID:\u{a0}01234 567890\nGeburtsdatum: 01.02.1990"
                    .into(),
            }],
        }
    }
    fn fact(property: &str, value: &str, quote: &str) -> ExtractedFact {
        ExtractedFact {
            property: property.into(),
            value: value.into(),
            quote: quote.into(),
            segment_id: "s1".into(),
            subject_quote: "Erika Beispiel".into(),
            context_quote: String::new(),
        }
    }
    #[test]
    fn whitespace_grouped_numbers_and_normalized_dates_recover_exact_source_slices() {
        let result = ground_extraction(
            &input(),
            ExtractionOutput {
                facts: vec![
                    fact("person.tax_id", "01234567890", "Steuer-ID: 01234567890"),
                    fact(
                        "person.birth_date",
                        "1990-02-01",
                        "Geburtsdatum: 01.02.1990",
                    ),
                ],
            },
        );
        assert!(result.rejected.is_empty());
        assert_eq!(result.repaired, 2);
        assert_eq!(result.output.facts[0].value, "01234 567890");
        assert_eq!(result.output.facts[0].subject_quote, "Erika\nBeispiel");
        assert_eq!(result.output.facts[1].value, "01.02.1990");
        let mut short = input();
        short.segments[0].text = "Erika Beispiel\nGeburtsdatum: 1.2.1990".into();
        let checked = ground_extraction(
            &short,
            ExtractionOutput {
                facts: vec![fact("person.birth_date", "1990-02-01", "1.2.1990")],
            },
        );
        assert!(checked.rejected.is_empty());
        assert_eq!(checked.output.facts[0].value, "1.2.1990");
    }
    #[test]
    fn missing_owner_retains_exact_source_values_for_confirmation_without_hiding_bad_evidence() {
        let mut candidate = fact("person.tax_id", "01234567890", "Steuer-ID: 01234567890");
        candidate.subject_quote.clear();
        let mut fabricated = candidate.clone();
        fabricated.quote = "Invented 01234567890".into();
        let mut context = candidate.clone();
        context.context_quote = "Not in the source".into();
        let result = ground_extraction(
            &input(),
            ExtractionOutput {
                facts: vec![candidate.clone(), candidate, fabricated, context],
            },
        );
        assert!(result.output.facts.is_empty());
        assert_eq!(result.rejected.len(), 3);
        let question = &result.rejected[0];
        assert_eq!(question.code, "subject_unknown");
        assert_eq!(question.fact.value, "01234 567890");
        assert_eq!(question.fact.quote, "Steuer-ID:\u{a0}01234 567890");
        assert!(question.fact.subject_quote.is_empty());
        assert_eq!(result.rejected[1].code, "quote_not_in_segment");
        assert_eq!(result.rejected[2].code, "context_not_in_source");
    }
    #[test]
    fn fabricated_quotes_values_subjects_and_source_ids_are_rejected_individually() {
        let good = fact("person.birth_date", "01.02.1990", "01.02.1990");
        let mut bad_subject = good.clone();
        bad_subject.subject_quote = "Other person".into();
        let mut bad_source = good.clone();
        bad_source.segment_id = "other-source".into();
        let result = ground_extraction(
            &input(),
            ExtractionOutput {
                facts: vec![
                    good,
                    fact("person.tax_id", "01234567890", "Invented 01234567890"),
                    fact("person.birth_date", "1991-02-01", "01.02.1990"),
                    bad_subject,
                    bad_source,
                ],
            },
        );
        assert_eq!(result.output.facts.len(), 1);
        assert_eq!(result.rejected.len(), 4);
    }
}
