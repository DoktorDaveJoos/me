//! Types grounded reader values. The reader quotes what is printed; code decides
//! whether the quote holds a date, an amount in which currency, a checksum-valid
//! identifier or a category. Nothing here calculates or guesses.
use crate::{
    Candidate, CandidateKind, CandidateValue, SourceSegment, candidates, doc_types::ValueKind,
    find_candidates,
};

pub struct TypingContext {
    /// Currency for bare amounts: the document's single currency, else the type default.
    pub currency: Option<&'static str>,
    /// The document prints separate EUR and Ct columns.
    pub euro_cent: bool,
    pub year: i32,
}
impl TypingContext {
    pub fn new(segments: &[SourceSegment<'_>], doc_type: Option<&str>, year: i32) -> Self {
        Self {
            currency: document_currency(segments)
                .or_else(|| doc_type.and_then(crate::default_currency)),
            euro_cent: segments
                .iter()
                .any(|s| candidates::has_euro_cent_columns(s.text)),
            year,
        }
    }
}

/// The single currency a document prints anywhere, if exactly one.
pub fn document_currency(segments: &[SourceSegment<'_>]) -> Option<&'static str> {
    let mut all = std::collections::BTreeSet::new();
    for s in segments {
        all.extend(candidates::currencies_in(s.text));
    }
    (all.len() == 1).then(|| all.into_iter().next()).flatten()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Located {
    pub segment_id: String,
    /// Absolute byte span of the value inside the segment.
    pub start: usize,
    pub end: usize,
    /// Absolute byte span of the whole quote inside the segment.
    pub quote_start: usize,
    pub quote_end: usize,
    /// The trimmed source line(s) of the quote.
    pub line: String,
}

/// Where a grounded fact sits. `None` when the quote or value is not verbatim.
pub fn locate(
    segments: &[SourceSegment<'_>],
    segment_id: &str,
    quote: &str,
    value: &str,
) -> Option<Located> {
    let text = segments.iter().find(|s| s.id == segment_id)?.text;
    let quote_start = text.find(quote)?;
    let offset = quote.find(value)?;
    let start = quote_start + offset;
    let end = start + value.len();
    let quote_end = quote_start + quote.len();
    Some(Located {
        segment_id: segment_id.to_owned(),
        start,
        end,
        quote_start,
        quote_end,
        line: candidates::line_window_for(text, start, end),
    })
}

fn scan_value(value: &str, year: i32) -> Vec<Candidate> {
    find_candidates(
        &[SourceSegment {
            id: "v",
            text: value,
        }],
        &[],
        year,
    )
}

/// Amount and currency of a printed value; the currency is `None` when neither
/// the value, the document nor the type states one.
pub fn amount_of(ctx: &TypingContext, value: &str) -> Option<(String, Option<String>)> {
    let found = scan_value(value, ctx.year);
    if let Some(CandidateValue::Money { amount, currency }) = found
        .iter()
        .find(|c| c.kind == CandidateKind::Money && c.text.trim() == value.trim())
        .map(|c| c.value.clone())
    {
        return Some((amount, Some(currency)));
    }
    let amount = candidates::parse_bare_amount(value, ctx.euro_cent, true)?;
    Some((amount, ctx.currency.map(str::to_owned)))
}

/// Maps a printed category such as `1`, `III` or `Steuerklasse IV` to its key.
pub fn category_key(
    options: &'static [(&'static str, &'static str)],
    value: &str,
) -> Option<&'static str> {
    const ROMAN: [&str; 6] = ["I", "II", "III", "IV", "V", "VI"];
    let upper = value.trim().to_uppercase();
    if let Some((key, _)) = options.iter().find(|(k, _)| k.to_uppercase() == upper) {
        return Some(key);
    }
    let token = upper
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .find(|t| t.parse::<usize>().is_ok() || ROMAN.contains(t))?;
    let roman = match token.parse::<usize>() {
        Ok(n @ 1..=6) => ROMAN[n - 1],
        Ok(_) => return None,
        Err(_) => ROMAN.iter().find(|r| **r == token)?,
    };
    options.iter().find(|(k, _)| *k == roman).map(|(k, _)| *k)
}

/// A candidate for `kind` at the value's exact span, or `None` when the printed
/// value cannot hold that kind.
pub fn type_value(
    ctx: &TypingContext,
    at: &Located,
    label: &str,
    value: &str,
    kind: ValueKind,
) -> Option<Candidate> {
    let found = scan_value(value, ctx.year);
    let whole = |k: CandidateKind| {
        found
            .iter()
            .find(|c| c.kind == k && c.text.trim() == value.trim())
            .cloned()
    };
    let (kind_out, typed, checksum) = match kind {
        ValueKind::Money(_) | ValueKind::Balance => {
            let (amount, currency) = amount_of(ctx, value)?;
            (
                CandidateKind::Money,
                CandidateValue::Money {
                    amount,
                    currency: currency?,
                },
                false,
            )
        }
        ValueKind::Date => {
            let c = whole(CandidateKind::Date)?;
            (CandidateKind::Date, c.value, false)
        }
        ValueKind::Period(granularity) => {
            // A year is a period only after a printed keyword ("Veranlagungszeitraum
            // 2025"), so a bare year value is typed in its source line.
            let c = whole(CandidateKind::Period).or_else(|| {
                scan_value(&at.line, ctx.year)
                    .into_iter()
                    .find(|c| c.kind == CandidateKind::Period && c.text.trim() == value.trim())
            })?;
            // The printed span must match the slot's own granularity: a year must
            // never type into a month slot (`pay_month`) and a month must never
            // type into a year slot (`tax_year`).
            match &c.value {
                CandidateValue::Period { start, end }
                    if candidates::whole_period(granularity, start, end) => {}
                _ => return None,
            }
            (CandidateKind::Period, c.value, false)
        }
        ValueKind::Identifier => {
            let checked = found.iter().find(|c| {
                c.checksum
                    && matches!(
                        c.kind,
                        CandidateKind::TaxId
                            | CandidateKind::SocialInsuranceNumber
                            | CandidateKind::Iban
                    )
            });
            match checked {
                Some(c) => (CandidateKind::Identifier, c.value.clone(), true),
                None => {
                    let text = value.trim();
                    if text.is_empty() {
                        return None;
                    }
                    (
                        CandidateKind::Identifier,
                        CandidateValue::Identifier(text.to_owned()),
                        false,
                    )
                }
            }
        }
        ValueKind::Category(options) => {
            let key = category_key(options, value)?;
            (
                CandidateKind::LabelValue,
                CandidateValue::Text(key.to_owned()),
                false,
            )
        }
        ValueKind::Text | ValueKind::Name => {
            let text = value.trim();
            if text.is_empty() {
                return None;
            }
            (
                CandidateKind::LabelValue,
                CandidateValue::Text(text.to_owned()),
                false,
            )
        }
    };
    Some(Candidate {
        id: String::new(),
        kind: kind_out,
        text: value.to_owned(),
        value: typed,
        segment_id: at.segment_id.clone(),
        start: at.start,
        end: at.end,
        line: at.line.clone(),
        label: Some(candidates::truncate_bytes(label, candidates::LABEL_MAX).to_owned()),
        checksum,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CandidateKind as K, CandidateValue as V, Period};

    const PAYSLIP: &str = include_str!("../fixtures/documents/payslip_datev_rows.txt");
    const LSTB: &str = include_str!("../fixtures/documents/lstb_2026.txt");

    fn seg(text: &str) -> Vec<SourceSegment<'_>> {
        vec![SourceSegment { id: "s0", text }]
    }

    #[test]
    fn currency_comes_from_the_document_then_the_type() {
        assert_eq!(document_currency(&seg(PAYSLIP)), Some("EUR"));
        assert_eq!(document_currency(&seg("Betrag 12,00\n")), None);
        assert_eq!(document_currency(&seg("12,00 EUR und 5,00 CHF\n")), None);
        let ctx = TypingContext::new(&seg("Betrag 12,00\n"), Some("payslip"), 2026);
        assert_eq!(ctx.currency, Some("EUR"));
        let ctx = TypingContext::new(&seg("Betrag 12,00\n"), Some("invoice"), 2026);
        assert_eq!(ctx.currency, None);
    }

    #[test]
    fn a_grounded_payslip_amount_becomes_money_at_its_exact_span() {
        let segments = seg(PAYSLIP);
        let ctx = TypingContext::new(&segments, Some("payslip"), 2026);
        let at = locate(&segments, "s0", "Lohnsteuer   1.032,58", "1.032,58").unwrap();
        assert_eq!(&PAYSLIP[at.start..at.end], "1.032,58");
        let c = type_value(
            &ctx,
            &at,
            "Lohnsteuer",
            "1.032,58",
            ValueKind::Money(Some(Period::Month)),
        )
        .unwrap();
        assert_eq!(c.kind, K::Money);
        assert_eq!(
            c.value,
            V::Money {
                amount: "1032.58".into(),
                currency: "EUR".into()
            }
        );
        assert_eq!(c.label.as_deref(), Some("Lohnsteuer"));
    }

    #[test]
    fn euro_cent_cells_and_whole_amounts_type_for_money_slots() {
        let segments = seg(LSTB);
        let ctx = TypingContext::new(&segments, Some("wage_tax_certificate"), 2026);
        assert!(ctx.euro_cent);
        let quote = "4. Einbehaltene Lohnsteuer von 3.   12.390   96";
        let at = locate(&segments, "s0", quote, "12.390   96").unwrap();
        let c = type_value(
            &ctx,
            &at,
            "4. Einbehaltene Lohnsteuer von 3.",
            "12.390   96",
            ValueKind::Money(Some(Period::Year)),
        )
        .unwrap();
        assert_eq!(
            c.value,
            V::Money {
                amount: "12390.96".into(),
                currency: "EUR".into()
            }
        );
        let whole = seg("Festgesetzt: Einkommensteuer 4.200\n");
        let ctx = TypingContext::new(&whole, Some("tax_assessment"), 2026);
        let at = locate(&whole, "s0", "Einkommensteuer 4.200", "4.200").unwrap();
        let c = type_value(
            &ctx,
            &at,
            "Einkommensteuer",
            "4.200",
            ValueKind::Money(Some(Period::Year)),
        )
        .unwrap();
        assert_eq!(
            c.value,
            V::Money {
                amount: "4200".into(),
                currency: "EUR".into()
            }
        );
    }

    #[test]
    fn values_that_cannot_hold_the_slot_kind_are_rejected() {
        let segments = seg(PAYSLIP);
        let ctx = TypingContext::new(&segments, Some("payslip"), 2026);
        let at = locate(&segments, "s0", "Krankenkasse   AOK Bayern", "AOK Bayern").unwrap();
        assert!(
            type_value(
                &ctx,
                &at,
                "Krankenkasse",
                "AOK Bayern",
                ValueKind::Money(None)
            )
            .is_none()
        );
        assert!(type_value(&ctx, &at, "Krankenkasse", "AOK Bayern", ValueKind::Date).is_none());
        let none = TypingContext {
            currency: None,
            euro_cent: false,
            year: 2026,
        };
        let at = locate(&segments, "s0", "Lohnsteuer   1.032,58", "1.032,58").unwrap();
        assert!(type_value(&none, &at, "Lohnsteuer", "1.032,58", ValueKind::Money(None)).is_none());
    }

    #[test]
    fn identifiers_keep_checksums_and_categories_map_printed_classes() {
        let segments = seg(PAYSLIP);
        let ctx = TypingContext::new(&segments, Some("payslip"), 2026);
        let at = locate(&segments, "s0", "Steuer-ID   65929970489", "65929970489").unwrap();
        let c = type_value(&ctx, &at, "Steuer-ID", "65929970489", ValueKind::Identifier).unwrap();
        assert_eq!(c.value, V::Identifier("65929970489".into()));
        assert!(c.checksum);
        assert_eq!(category_key(crate::TAX_CLASSES, "1"), Some("I"));
        assert_eq!(
            category_key(crate::TAX_CLASSES, "Steuerklasse III"),
            Some("III")
        );
        assert_eq!(category_key(crate::TAX_CLASSES, "IV/Faktor"), Some("IV"));
        assert_eq!(category_key(crate::TAX_CLASSES, "7"), None);
    }

    #[test]
    fn periods_type_from_month_values() {
        let text = "Abrechnungsmonat Januar 2026\n";
        let segments = seg(text);
        let ctx = TypingContext::new(&segments, Some("payslip"), 2026);
        let at = locate(
            &segments,
            "s0",
            "Abrechnungsmonat Januar 2026",
            "Januar 2026",
        )
        .unwrap();
        let c = type_value(
            &ctx,
            &at,
            "Abrechnungsmonat",
            "Januar 2026",
            ValueKind::Period(Period::Month),
        )
        .unwrap();
        assert_eq!(
            c.value,
            V::Period {
                start: "2026-01-01".into(),
                end: "2026-01-31".into()
            }
        );
    }

    #[test]
    fn a_year_types_as_a_period_only_in_its_printed_period_context() {
        let text = "Veranlagungszeitraum   2025\nKundennummer   2025\n";
        let segments = seg(text);
        let ctx = TypingContext::new(&segments, Some("tax_assessment"), 2026);
        let at = locate(&segments, "s0", "Veranlagungszeitraum   2025", "2025").unwrap();
        let c = type_value(
            &ctx,
            &at,
            "Veranlagungszeitraum",
            "2025",
            ValueKind::Period(Period::Year),
        )
        .unwrap();
        assert_eq!(
            c.value,
            V::Period {
                start: "2025-01-01".into(),
                end: "2025-12-31".into()
            }
        );
        assert_eq!((c.start, c.end), (at.start, at.end));
        // A bare year without a period keyword is not a period.
        let at = locate(&segments, "s0", "Kundennummer   2025", "2025").unwrap();
        assert!(
            type_value(
                &ctx,
                &at,
                "Kundennummer",
                "2025",
                ValueKind::Period(Period::Year)
            )
            .is_none()
        );
    }

    #[test]
    fn period_granularity_guards_month_and_year_slots() {
        // `pay_month` accepts only a month period: a year in period context
        // (e.g. "Kalenderjahr 2026") must never type into it.
        let text = "Kalenderjahr 2026\nAbrechnung Februar 2026\n";
        let segments = seg(text);
        let ctx = TypingContext::new(&segments, Some("payslip"), 2026);
        let at = locate(&segments, "s0", "Kalenderjahr 2026", "2026").unwrap();
        assert!(
            type_value(
                &ctx,
                &at,
                "Kalenderjahr",
                "2026",
                ValueKind::Period(Period::Month)
            )
            .is_none()
        );
        let at = locate(&segments, "s0", "Abrechnung Februar 2026", "Februar 2026").unwrap();
        let c = type_value(
            &ctx,
            &at,
            "Abrechnung",
            "Februar 2026",
            ValueKind::Period(Period::Month),
        )
        .unwrap();
        assert_eq!(
            c.value,
            V::Period {
                start: "2026-02-01".into(),
                end: "2026-02-28".into()
            }
        );

        // `tax_year` accepts only a whole-year period: a month must never type
        // into it.
        let text = "Monat Januar 2025\nVeranlagungszeitraum 2025\n";
        let segments = seg(text);
        let ctx = TypingContext::new(&segments, Some("tax_assessment"), 2026);
        let at = locate(&segments, "s0", "Monat Januar 2025", "Januar 2025").unwrap();
        assert!(
            type_value(
                &ctx,
                &at,
                "Monat",
                "Januar 2025",
                ValueKind::Period(Period::Year)
            )
            .is_none()
        );
        let at = locate(&segments, "s0", "Veranlagungszeitraum 2025", "2025").unwrap();
        let c = type_value(
            &ctx,
            &at,
            "Veranlagungszeitraum",
            "2025",
            ValueKind::Period(Period::Year),
        )
        .unwrap();
        assert_eq!(
            c.value,
            V::Period {
                start: "2025-01-01".into(),
                end: "2025-12-31".into()
            }
        );
    }

    #[test]
    fn locate_windows_a_long_multi_byte_line_by_bytes_around_the_value() {
        let text = format!(
            "{} Lohnsteuer   1.032,58 {}",
            "ü".repeat(500),
            "ü".repeat(500)
        );
        let segments = seg(&text);
        let at = locate(&segments, "s0", "Lohnsteuer   1.032,58", "1.032,58").unwrap();
        assert!(at.line.len() <= 240, "line was {} bytes", at.line.len());
        assert!(at.line.contains("1.032,58"));
    }

    #[test]
    fn type_value_truncates_a_long_multi_byte_label_by_bytes() {
        let segments = seg(PAYSLIP);
        let ctx = TypingContext::new(&segments, Some("payslip"), 2026);
        let at = locate(&segments, "s0", "Lohnsteuer   1.032,58", "1.032,58").unwrap();
        let label = "ä".repeat(200);
        let c = type_value(
            &ctx,
            &at,
            &label,
            "1.032,58",
            ValueKind::Money(Some(Period::Month)),
        )
        .unwrap();
        let got = c.label.unwrap();
        assert!(got.len() <= 80, "label was {} bytes", got.len());
        assert!(label.starts_with(&got));
    }

    #[test]
    fn a_values_own_printed_currency_beats_the_types_default() {
        let text = "Betrag 12,00 CHF\n";
        let segments = seg(text);
        let ctx = TypingContext::new(&segments, Some("payslip"), 2026);
        let at = locate(&segments, "s0", "Betrag 12,00 CHF", "12,00 CHF").unwrap();
        let c = type_value(&ctx, &at, "Betrag", "12,00 CHF", ValueKind::Money(None)).unwrap();
        assert_eq!(
            c.value,
            V::Money {
                amount: "12.00".into(),
                currency: "CHF".into()
            }
        );
    }
}
