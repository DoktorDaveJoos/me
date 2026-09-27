# Deep-first Extraction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every imported document is read in full by the grounded reader, every reader assumption is verified by TypeSafe, confident values reach the profile silently, doubtful ones become Quick checks, and nothing the local scanners see is lost silently.

**Architecture:** The local scanner learns bare amounts, EUR/Ct cells and month/year periods (`me-core`). The registry becomes a checklist with new tax values. The deep reader (`me-agent` Codex pipeline) receives a family/type guide plus the checklist and tags facts with slots. Code types each grounded value, sweeps for omissions, and asks TypeSafe batched verification questions. A new `me-core` read store persists profile assertions (through the existing Resolve), document facts and not-interpreted values. The desktop runs one pipeline for automatic and manual reads and shows the three groups.

**Tech Stack:** Rust 2024 workspace, GPUI (pinned), rusqlite + SQLCipher, serde_json, TypeSafe HTTP decisions (`jev-1.13.0`), Codex app-server reader (`gpt-5.6-sol`).

Spec: [docs/superpowers/specs/2026-09-27-deep-first-extraction-design.md](../specs/2026-09-27-deep-first-extraction-design.md)

## Global Constraints

- Run every command from the repository root. Use `./scripts/cargo` (Cargo is not on PATH).
- Keep blocking I/O, crypto and network work off the UI thread; hold the vault mutex only for short calls, never across provider requests.
- Never log real personal data, labels, values or file names. Diagnostics carry counts and codes only.
- Never ship or read the shared OpenAI key. TypeSafe keys come only from `TYPESAFE_API_KEY` / the private `.env`.
- Money always has a three-letter uppercase currency or does not become a profile value. A currency is never guessed: document marker, else type default (`EUR` for `payslip`, `wage_tax_certificate`, `tax_assessment`), else none.
- Two-digit years are never expanded outside MRZ rules.
- Code owns dates, arithmetic and signs; TypeSafe answers narrow typed questions; the reader never calculates.
- All TypeSafe question texts and thresholds live in `crates/me-agent/src/typesafe_questions.rs`.
- Bands: `failure > VERIFY_FIRE (0.7)` or `confidence < SLOT_FLOOR (0.6)` → reject; `confidence < threshold` (vault `check_threshold`, 0.7–0.9) → check; else accept.
- User decisions are never overridden. Graph policy becomes `import-graph-v2`.
- UI uses only design-system tokens (`space::*`, `radius::STANDARD`, `type_style(Type::…)`, semantic colors, `font::MONO` for values). No local hex, sizes or radii.
- Before finishing Rust changes: `./scripts/cargo fmt --all`, `./scripts/cargo clippy --workspace --all-targets --all-features -- -D warnings`, relevant tests; for UI tasks also `./scripts/check-design-system` and `python3 scripts/test-design-system.py`.
- Preserve `publish = false`, the license and third-party notices. Commit `Cargo.lock` if it changes.
- End every commit message with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Work on the current branch `feat/honeycomb-depth-import-graph`.

## File Map

| File | Responsibility | Tasks |
| --- | --- | --- |
| `crates/me-core/src/candidates.rs` | Local scanners: new `Amount`, `Period` kinds, EUR/Ct cells, `parse_bare_amount`, `currencies_in`, `has_euro_cent_columns` | 1 |
| `crates/me-core/fixtures/documents/*.txt` | Synthetic document corpus (no personal data) | 1, 12 |
| `crates/me-core/src/doc_types.rs` | Registry v2: `ValueKind::Period`, `ValueKind::Balance`, tax slots, `TAX_CLASSES`, `default_currency` | 2 |
| `crates/me-core/src/vocabulary.rs` | New `person.*` tax properties | 2 |
| `crates/me-core/src/typing.rs` (new) | `TypingContext`, `locate`, `type_value`, `document_currency`, `category_key` | 3 |
| `crates/me-core/src/coverage.rs` (new) | Omission sweep: `Uncovered`, `uncovered` | 4 |
| `crates/me-core/migrations/016_document_read.sql` (new) | `document_fact`, `read_summary`, re-queue of v1 reads | 5 |
| `crates/me-core/src/reading.rs` (new) | `DocumentRead`, `ReadFact`, `Vault::apply_read`, `complete_read_run`, `document_read`, `read_self_check` | 5, 9, 11 |
| `crates/me-core/src/graph.rs` | Policy v2, period fallback, period-aware conflicts, corrections, assertion ids in outcome | 5 |
| `crates/me-core/src/knowledge.rs`, `grounding.rs` | `ExtractedFact.slot` | 6 |
| `crates/me-agent/src/guides.rs` (new) | `ReadingGuide`, `ChecklistItem`, `reading_guide`, family/type guide texts | 6 |
| `crates/me-agent/src/codex*.rs` | Guided reader, slot schema, subject-unknown checkpoints, `audit_document` | 6 |
| `crates/me-agent/src/fact_verification.rs` (new) | Batched TypeSafe verification, bands, competing mappings | 7 |
| `crates/me-agent/src/typesafe_questions.rs` | Fact questions, removal of slot-selection/gap-fill questions | 7, 8 |
| `crates/me-agent/src/graph_pipeline.rs` | Keep classify/search/equivalence/alignment; remove extract/gap fill | 8 |
| `apps/desktop/src/read_import.rs` (new, replaces `graph_import.rs`) | One read pipeline for automatic and manual runs | 8 |
| `apps/desktop/src/ai_ui.rs` | Call `run_read`; load read view | 8, 9 |
| `apps/desktop/src/vault_ui.rs` | "Read from this document" groups | 9 |
| `apps/desktop/src/import_ui.rs`, `crates/me-core/src/imports.rs` | Read counts line, Not read yet / Partly read | 10 |
| `apps/desktop/src/development_ui.rs` | Copy extraction self-check | 11 |
| `crates/me-agent/src/live_eval_tests.rs` (new) | Opt-in live corpus evaluation | 12 |
| `docs/import-pipeline.md`, spec, `docs/development.md` | Verified behavior and limits | 12 |

---

### Task 1: Bare amounts, EUR/Ct cells and period tokens in the local scanner

**Files:**
- Modify: `crates/me-core/src/candidates.rs`
- Modify: `crates/me-core/src/graph.rs:294-332` (`display_content`, `fact_value` arms)
- Modify: `crates/me-agent/src/typesafe_questions.rs:170-178` (`kind_name`)
- Create: `crates/me-core/fixtures/documents/payslip_datev_rows.txt`
- Create: `crates/me-core/fixtures/documents/lstb_2026.txt`

**Interfaces:**
- Produces: `CandidateKind::Amount`, `CandidateKind::Period`; `CandidateValue::Amount(String)` (exact decimal, no currency) and `CandidateValue::Period { start: String, end: String }` (ISO dates, inclusive); `pub fn parse_bare_amount(text: &str, euro_cent: bool, allow_whole: bool) -> Option<String>`; `pub(crate) fn currencies_in(text: &str) -> std::collections::BTreeSet<&'static str>`; `pub(crate) fn has_euro_cent_columns(text: &str) -> bool`.

- [ ] **Step 1: Add the synthetic fixtures**

`crates/me-core/fixtures/documents/payslip_datev_rows.txt` (invented values, OCR-row layout):

```text
Muster Software GmbH   Musterweg 1   80331 München
Abrechnung der Brutto/Netto-Bezüge für Januar 2026
Pers.-Nr.   00042   Eintritt   01.04.2021
Herrn   Max Beispiel   Beispielstraße 12   10115 Berlin
Steuer-ID   65929970489   StKl   1   Kfb   0,0   Konf   --
SV-Nummer   65 170839 J 003   Krankenkasse   AOK Bayern
KV-Zusatzbeitrag   2,50 %   Faktor   0,950
Lohnart   Bezeichnung   Betrag
1000   Gehalt   5.250,00
1300   Fahrtkostenzuschuss   90,00
Gesamt-Brutto   5.340,00
Steuer-Brutto   5.340,00   KV-Brutto   5.340,00   RV-Brutto   5.340,00
Lohnsteuer   1.032,58
Solidaritätszuschlag   0,00
Kirchensteuer   0,00
KV-Beitrag   435,21
RV-Beitrag   496,62
AV-Beitrag   69,42
PV-Beitrag   96,12
Netto-Verdienst   3.210,05
Auszahlungsbetrag   3.210,05
Überweisung an DE89 3704 0044 0532 0130 00   3.210,05 EUR
Jahreswerte   Steuer-Brutto   5.340,00   Lohnsteuer   1.032,58
```

`crates/me-core/fixtures/documents/lstb_2026.txt` (BMF 2026 layout, split EUR/Ct columns, invented values):

```text
Ausdruck der elektronischen Lohnsteuerbescheinigung für 2026
Nachstehende Daten wurden maschinell an die Finanzverwaltung übermittelt.
Identifikationsnummer:   65929970489
Personalnummer:   00042
Steuerklasse/Faktor   1
1. Bescheinigungszeitraum   01.01.2026 - 31.12.2026
EUR   Ct
3. Bruttoarbeitslohn einschl. Sachbezüge   64.080   00
4. Einbehaltene Lohnsteuer von 3.   12.390   96
5. Einbehaltener Solidaritätszuschlag von 3.   0   00
6. Einbehaltene Kirchensteuer des Arbeitnehmers von 3.   0   00
22. a) Arbeitgeberanteil zur gesetzlichen Rentenversicherung   5.959   44
23. a) Arbeitnehmeranteil zur gesetzlichen Rentenversicherung   5.959   44
25. Arbeitnehmerbeiträge zur gesetzlichen Krankenversicherung   5.222   52
26. Arbeitnehmerbeiträge zur sozialen Pflegeversicherung   1.153   44
27. Arbeitnehmerbeiträge zur Arbeitslosenversicherung   833   04
Anschrift und Steuernummer des Arbeitgebers:   Muster Software GmbH   Musterweg 1   80331 München
```

- [ ] **Step 2: Write the failing tests** (append inside `mod tests` in `candidates.rs`)

```rust
    fn scan(text: &str) -> Vec<Candidate> {
        find_candidates(&[SourceSegment { id: "s0", text }], &[], 2026)
    }
    fn labeled_amount(found: &[Candidate], label: &str) -> Option<String> {
        found
            .iter()
            .find(|c| c.kind == CandidateKind::Amount && c.label.as_deref() == Some(label))
            .map(|c| match &c.value {
                CandidateValue::Amount(a) => a.clone(),
                other => panic!("unexpected {other:?}"),
            })
    }

    #[test]
    fn bare_payroll_amounts_become_labeled_amounts() {
        let found = scan(include_str!("../fixtures/documents/payslip_datev_rows.txt"));
        assert_eq!(labeled_amount(&found, "Gesamt-Brutto").as_deref(), Some("5340.00"));
        assert_eq!(labeled_amount(&found, "Lohnsteuer").as_deref(), Some("1032.58"));
        assert_eq!(labeled_amount(&found, "Netto-Verdienst").as_deref(), Some("3210.05"));
        assert_eq!(labeled_amount(&found, "Solidaritätszuschlag").as_deref(), Some("0.00"));
        // A currency beside the value stays money and is never duplicated as an amount.
        assert!(found.iter().any(|c| c.kind == CandidateKind::Money && c.text == "3.210,05 EUR"));
        assert!(!found.iter().any(|c| c.kind == CandidateKind::Amount && c.text.contains("EUR")));
        // Percentages and three-decimal factors are not amounts.
        assert!(!found.iter().any(|c| c.kind == CandidateKind::Amount && c.text == "2,50"));
        assert!(!found.iter().any(|c| c.kind == CandidateKind::Amount && c.text == "0,950"));
        // The label/value catch-all at the same span hands its label over.
        assert!(!found.iter().any(|c| c.kind == CandidateKind::LabelValue && c.text == "1.032,58"));
    }

    #[test]
    fn euro_and_cent_cells_join_only_under_an_eur_ct_header() {
        let found = scan(include_str!("../fixtures/documents/lstb_2026.txt"));
        let amounts: Vec<(&str, &CandidateValue)> = found
            .iter()
            .filter(|c| c.kind == CandidateKind::Amount)
            .map(|c| (c.text.as_str(), &c.value))
            .collect();
        assert!(amounts.contains(&("64.080   00", &CandidateValue::Amount("64080.00".into()))));
        assert!(amounts.contains(&("12.390   96", &CandidateValue::Amount("12390.96".into()))));
        assert!(amounts.contains(&("833   04", &CandidateValue::Amount("833.04".into()))));
        // Without the header, two separate numbers stay separate.
        let plain = scan("Kostenstelle   4711   12\n");
        assert!(!plain.iter().any(|c| c.kind == CandidateKind::Amount));
    }

    #[test]
    fn month_and_year_periods_are_complete_and_never_part_of_a_date() {
        let period = |text: &str| -> Vec<(String, String)> {
            scan(text)
                .into_iter()
                .filter_map(|c| match c.value {
                    CandidateValue::Period { start, end } => Some((start, end)),
                    _ => None,
                })
                .collect()
        };
        let jan = vec![("2026-01-01".to_owned(), "2026-01-31".to_owned())];
        assert_eq!(period("Abrechnungsmonat Januar 2026\n"), jan);
        assert_eq!(period("Monat: Jan. 2026\n"), jan);
        assert_eq!(period("Zeitraum 01/2026\n"), jan);
        assert_eq!(period("Zeitraum 01.2026\n"), jan);
        assert_eq!(
            period("Periode 2024-02\n"),
            vec![("2024-02-01".to_owned(), "2024-02-29".to_owned())]
        );
        assert_eq!(
            period("Veranlagungszeitraum 2025\n"),
            vec![("2025-01-01".to_owned(), "2025-12-31".to_owned())]
        );
        assert_eq!(
            period("Lohnsteuerbescheinigung für 2026\n"),
            vec![("2026-01-01".to_owned(), "2026-12-31".to_owned())]
        );
        assert!(period("Datum 15. Januar 2026\n").is_empty());
        assert!(period("Datum 31.01.2026\n").is_empty());
        assert!(period("Datum 2026-01-15\n").is_empty());
        assert!(period("Monat 13/2026\n").is_empty());
    }

    #[test]
    fn parse_bare_amount_accepts_only_complete_amounts() {
        assert_eq!(parse_bare_amount(" 1.032,58 ", false, false).as_deref(), Some("1032.58"));
        assert_eq!(parse_bare_amount("4.200", false, false), None);
        assert_eq!(parse_bare_amount("4.200", false, true).as_deref(), Some("4200"));
        assert_eq!(parse_bare_amount("64.080   00", true, false).as_deref(), Some("64080.00"));
        assert_eq!(parse_bare_amount("64.080   00", false, false), None);
        assert_eq!(parse_bare_amount("1.032,58 Euro", false, true), None);
        assert_eq!(parse_bare_amount("-12,50", false, false).as_deref(), Some("-12.50"));
    }
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `./scripts/cargo test -p me-core --lib candidates::tests`
Expected: compile errors (`CandidateKind::Amount`, `CandidateValue::Period`, `parse_bare_amount` not found).

- [ ] **Step 4: Implement the kinds and scanners**

In `CandidateKind` add `Amount,` after `Money,` and `Period,` after `Date,`. In `CandidateValue` add:

```rust
    /// Exact decimal amount printed without a currency marker (`-1234.56`). The
    /// currency is resolved later from the document or its type, never here.
    Amount(String),
    /// A month or year the document names, as its first and last day (inclusive).
    Period { start: String, end: String },
```

Update the module doc comment sentence "money always needs an explicit currency" to: "money needs an explicit currency; bare two-decimal amounts become `Amount` candidates whose currency is resolved later".

In `kind_priority` add `Period => 7,` and `Amount => 8,` (shift nothing else; ties are fine).

In `struct Doc` add `euro_cent: bool,` and compute it at the end of `Doc::new`:

```rust
        let euro_cent = has_euro_cent_columns(text);
        Self { text, lines, prev_nonempty, euro_cent }
```

Add the helpers (near the money section):

```rust
/// A table header with separate `EUR` and `Ct` columns (official forms such as
/// the Lohnsteuerbescheinigung print euros and cents in two cells).
pub(crate) fn has_euro_cent_columns(text: &str) -> bool {
    text.lines().any(|line| {
        let words: Vec<String> = line
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_ascii_lowercase)
            .collect();
        words.iter().any(|w| w == "eur") && words.iter().any(|w| w == "ct")
    })
}

/// Every explicit currency marker in `text` (symbols, ISO codes, "Euro").
pub(crate) fn currencies_in(text: &str) -> std::collections::BTreeSet<&'static str> {
    let mut found = std::collections::BTreeSet::new();
    for (i, _) in text.char_indices() {
        if let Some((code, _)) = currency_at(text, i) {
            found.insert(code);
        }
    }
    found
}

/// `52.345` followed by a separate two-digit cents cell that ends the row.
fn euro_cent_pair(s: &str, end: usize, amount: &str) -> Option<(usize, String)> {
    if amount.contains('.') {
        return None;
    }
    let b = s.as_bytes();
    let mut j = end;
    while j < b.len() && matches!(b[j], b' ' | b'\t') {
        j += 1;
    }
    if j == end || j + 2 > b.len() || !b[j].is_ascii_digit() || !b[j + 1].is_ascii_digit() {
        return None;
    }
    let stop = j + 2;
    s[stop..]
        .trim()
        .is_empty()
        .then(|| (stop, format!("{amount}.{}", &s[j..stop])))
}

/// A complete amount without a currency: two decimals, a whole number when
/// `allow_whole`, or a joined EUR/Ct cell pair when `euro_cent`.
pub fn parse_bare_amount(text: &str, euro_cent: bool, allow_whole: bool) -> Option<String> {
    let t = text.trim();
    let (end, amount) = parse_amount(t, 0)?;
    if end == t.len() {
        let decimals = amount.rsplit_once('.').map(|(_, f)| f.len());
        return match decimals {
            Some(2) => Some(amount),
            None if allow_whole => Some(amount),
            _ => None,
        };
    }
    if euro_cent {
        return euro_cent_pair(t, end, &amount).filter(|(stop, _)| *stop == t.len()).map(|(_, a)| a);
    }
    None
}

/// Bare amounts: exactly two decimals and no adjacent currency marker or `%`.
fn scan_amounts(ctx: &LineCtx<'_>, out: &mut Vec<Raw>) {
    let s = ctx.text;
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let signed = matches!(b[i], b'-' | b'+')
            && b.get(i + 1).is_some_and(u8::is_ascii_digit)
            && prev_char(s, i).is_none_or(|c| !c.is_alphanumeric());
        if !(signed || (b[i].is_ascii_digit() && number_start(s, i))) {
            i += 1;
            continue;
        }
        let Some((end, amount)) = parse_amount(s, i) else {
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            continue;
        };
        let after = skip_gap_forward(s, end);
        let marked = currency_at(s, after).is_some()
            || currency_before(s, skip_gap_backward(s, i)).is_some()
            || next_char(s, after) == Some('%');
        let two_decimals = amount.rsplit_once('.').is_some_and(|(_, f)| f.len() == 2);
        if !marked && two_decimals && number_end(s, end) {
            out.push(ctx.raw(CandidateKind::Amount, i, end, CandidateValue::Amount(amount)));
        } else if !marked
            && ctx.doc.euro_cent
            && let Some((stop, joined)) = euro_cent_pair(s, end, &amount)
        {
            out.push(ctx.raw(CandidateKind::Amount, i, stop, CandidateValue::Amount(joined)));
            i = stop;
            continue;
        }
        i = end.max(i + 1);
    }
}
```

Add the period scanner (after `scan_dates`):

```rust
const YEAR_KEYWORDS: &[&str] = &[
    "veranlagungszeitraum",
    "kalenderjahr",
    "steuerjahr",
    "jahr",
    "für",
    "fuer",
    "tax year",
];

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 31,
    }
}
fn month_period(year: i32, month: u32) -> Option<(String, String)> {
    if !(1..=12).contains(&month) {
        return None;
    }
    Some((
        iso_date(year, month, 1)?,
        iso_date(year, month, days_in_month(year, month))?,
    ))
}
/// "15. Januar 2026" is a date, not a month.
fn day_before(s: &str, i: usize) -> bool {
    let head = s[..i].trim_end();
    let head = head.strip_suffix('.').unwrap_or(head).trim_end();
    head.as_bytes().last().is_some_and(u8::is_ascii_digit)
}
/// `01/2026`, `01.2026` or `2026-01`, never part of a longer date.
fn numeric_month(s: &str, i: usize) -> Option<(usize, (String, String))> {
    let b = s.as_bytes();
    let (first, j) = read_digits(b, i, 1, 4)?;
    let sep = *b.get(j)?;
    if j - i == 4 {
        if sep != b'-' {
            return None;
        }
        let (month, end) = read_digits(b, j + 1, 2, 2)?;
        return number_end(s, end)
            .then(|| month_period(first as i32, month))
            .flatten()
            .map(|p| (end, p));
    }
    if !matches!(sep, b'/' | b'.') || j - i > 2 {
        return None;
    }
    let (year, end) = read_year(s, j + 1)?;
    month_period(year, first).map(|p| (end, p))
}
/// A four-digit year directly after a period keyword ("Veranlagungszeitraum 2025").
fn year_period(s: &str, i: usize) -> Option<(usize, (String, String))> {
    let (year, end) = read_year(s, i)?;
    let from = floor_boundary(s, i.saturating_sub(24));
    let before = s[from..i].to_lowercase();
    let before = before.trim_end().trim_end_matches(':').trim_end();
    YEAR_KEYWORDS
        .iter()
        .any(|k| before.ends_with(k))
        .then(|| (end, (format!("{year:04}-01-01"), format!("{year:04}-12-31"))))
}
fn scan_periods(ctx: &LineCtx<'_>, out: &mut Vec<Raw>) {
    let s = ctx.text;
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let parsed = if b[i].is_ascii_alphabetic() && word_start(s, i) && !day_before(s, i) {
            read_month(s, i).and_then(|(month, j)| {
                let (year, end) = read_year(s, skip_spaces(b, j, 3))?;
                month_period(year, month).map(|p| (end, p))
            })
        } else if b[i].is_ascii_digit() && number_start(s, i) {
            numeric_month(s, i).or_else(|| year_period(s, i))
        } else {
            None
        };
        match parsed {
            Some((end, (start, last))) => {
                out.push(ctx.raw(
                    CandidateKind::Period,
                    i,
                    end,
                    CandidateValue::Period { start, end: last },
                ));
                i = end;
            }
            None if b[i].is_ascii_alphanumeric() => {
                while i < b.len() && b[i].is_ascii_alphanumeric() {
                    i += 1;
                }
            }
            None => i += 1,
        }
    }
}
```

In `scan_document`, inside the digit block, add `scan_amounts(&ctx, &mut out);` after `scan_money` and `scan_periods(&ctx, &mut out);` after `scan_dates`.

In `clean_up`, `specific` already excludes only `LabelValue | Identifier | PersonName`, so `Amount` and `Period` spans remove duplicate label/value candidates and receive their labels. No change needed there.

- [ ] **Step 5: Handle the new values everywhere they are matched**

`crates/me-core/src/graph.rs` `display_content`:

```rust
            CandidateValue::Amount(a) => a.clone(),
            CandidateValue::Period { start, end } => format!("{start} – {end}"),
```

`fact_value` needs no change (it matches `Money` and `Date` explicitly and falls through).

`crates/me-agent/src/typesafe_questions.rs` `kind_name`:

```rust
        CandidateKind::Money | CandidateKind::Amount => "amount",
        CandidateKind::Period => "period",
```

- [ ] **Step 6: Run the tests and fix existing expectations**

Run: `./scripts/cargo test -p me-core --lib candidates::tests`
Expected: the four new tests PASS. If an existing test asserted a `LabelValue` for a bare two-decimal amount, change that assertion to `CandidateKind::Amount` (the span and label are unchanged); do not weaken other assertions.

Run: `./scripts/cargo test -p me-core && ./scripts/cargo test -p me-agent`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
./scripts/cargo fmt --all
git add crates/me-core/src/candidates.rs crates/me-core/src/graph.rs crates/me-agent/src/typesafe_questions.rs crates/me-core/fixtures
git commit -m "Read bare payroll amounts, EUR/Ct cells and month periods locally

Payroll and tax tables print the currency once, so the scanner dropped
every amount. Bare two-decimal amounts, joined EUR/Ct cells and month or
year periods are now typed candidates with exact spans.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Registry v2 with tax values, periods and balances

**Files:**
- Modify: `crates/me-core/src/doc_types.rs`
- Modify: `crates/me-core/src/vocabulary.rs`
- Modify: `crates/me-core/src/lib.rs` (re-export `default_currency`, `TAX_CLASSES`)

**Interfaces:**
- Consumes: `CandidateKind::Amount`, `CandidateKind::Period` (Task 1).
- Produces: `ValueKind::Period`, `ValueKind::Balance`; `pub const TAX_CLASSES: &[(&str, &str)]`; `pub fn default_currency(doc_type: &str) -> Option<&'static str>`; `REGISTRY_VERSION = "doc-types-v2"`; new slot keys listed below; properties `person.wage_tax`, `person.solidarity_surcharge`, `person.church_tax`, `person.pension_contribution`, `person.health_insurance_contribution`, `person.care_insurance_contribution`, `person.unemployment_insurance_contribution`, `person.taxable_income`, `person.income_tax_assessed`, `person.tax_balance` (money), `person.tax_payment_due` (date), `person.tax_class` (text).

- [ ] **Step 1: Write the failing tests** (append to `mod tests` in `doc_types.rs`)

```rust
    #[test]
    fn tax_documents_list_the_tax_checklist() {
        let keys = |t: &str| -> Vec<&str> { doc_type(t).unwrap().slots.iter().map(|s| s.key).collect() };
        let payslip = keys("payslip");
        for k in [
            "gross", "net", "wage_tax", "solidarity_surcharge", "church_tax", "tax_class",
            "pension_contribution", "health_contribution", "care_contribution",
            "unemployment_contribution", "pay_month",
        ] {
            assert!(payslip.contains(&k), "payslip lacks {k}");
        }
        let certificate = keys("wage_tax_certificate");
        for k in [
            "gross_wage", "wage_tax", "solidarity_surcharge", "church_tax",
            "pension_contribution", "health_contribution", "care_contribution",
            "unemployment_contribution",
        ] {
            assert!(certificate.contains(&k), "certificate lacks {k}");
        }
        let assessment = keys("tax_assessment");
        for k in ["tax_year", "taxable_income", "income_tax_assessed", "tax_balance", "payment_due"] {
            assert!(assessment.contains(&k), "assessment lacks {k}");
        }
        let balance = doc_type("tax_assessment").unwrap().slot("tax_balance").unwrap();
        assert_eq!(balance.value, ValueKind::Balance);
        assert_eq!(
            doc_type("payslip").unwrap().slot("wage_tax").unwrap().value,
            ValueKind::Money(Some(Period::Month))
        );
        assert_eq!(
            doc_type("wage_tax_certificate").unwrap().slot("wage_tax").unwrap().value,
            ValueKind::Money(Some(Period::Year))
        );
    }

    #[test]
    fn only_german_payroll_and_tax_forms_default_to_euro() {
        assert_eq!(default_currency("payslip"), Some("EUR"));
        assert_eq!(default_currency("wage_tax_certificate"), Some("EUR"));
        assert_eq!(default_currency("tax_assessment"), Some("EUR"));
        assert_eq!(default_currency("insurance_policy"), None);
        assert_eq!(default_currency("invoice"), None);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `./scripts/cargo test -p me-core --lib doc_types::tests`
Expected: compile errors (`ValueKind::Balance`, `default_currency`).

- [ ] **Step 3: Implement the registry changes**

1. `pub const REGISTRY_VERSION: &str = "doc-types-v2";`
2. Add to `ValueKind`:

```rust
    /// A month or year the document covers; code computes its first and last day.
    Period,
    /// A signed one-off amount whose direction (refund or payment) TypeSafe judges;
    /// code stores a refund as negative.
    Balance,
```

3. Constants and helpers after `const TEXT`:

```rust
const PERIOD: &[C] = &[C::Period];

/// Wage tax classes; printed as `1`–`6` or Roman numerals.
pub const TAX_CLASSES: &[(&str, &str)] = &[
    ("I", "Steuerklasse I: single, or separated/divorced/widowed without the relief"),
    ("II", "Steuerklasse II: single parent with the relief amount"),
    ("III", "Steuerklasse III: married, the higher-earning partner"),
    ("IV", "Steuerklasse IV: married, both partners earning similarly (also with factor)"),
    ("V", "Steuerklasse V: married, the partner of someone in class III"),
    ("VI", "Steuerklasse VI: a second or further employment"),
];

/// Legal currency of German payroll and tax forms, used only when the document
/// itself prints no single currency.
pub fn default_currency(doc_type: &str) -> Option<&'static str> {
    matches!(doc_type, "payslip" | "wage_tax_certificate" | "tax_assessment").then_some("EUR")
}

const fn employee_money(
    key: &'static str,
    property: &'static str,
    period: Period,
    label: &'static str,
    description: &'static str,
) -> Slot {
    valid(
        slot(key, Some(property), Target::Subject, ValueKind::Money(Some(period)), MONEY, label, description),
        SlotValidity::DocumentPeriod,
    )
}
```

   Change `const MONEY: &[C] = &[C::Money, C::Amount];`.

4. Payslip: insert after the `net` slot:

```rust
            employee_money("wage_tax", "person.wage_tax", Period::Month, "wage tax",
                "Lohnsteuer withheld from this month's pay, not the year-to-date total"),
            employee_money("solidarity_surcharge", "person.solidarity_surcharge", Period::Month,
                "solidarity surcharge", "Solidaritätszuschlag withheld this month"),
            employee_money("church_tax", "person.church_tax", Period::Month, "church tax",
                "Kirchensteuer withheld from the employee this month"),
            valid(
                slot("tax_class", Some("person.tax_class"), Target::Subject,
                    ValueKind::Category(TAX_CLASSES), &[], "tax class",
                    "The wage tax class (Steuerklasse, StKl) applied this month"),
                SlotValidity::DocumentPeriod,
            ),
            employee_money("pension_contribution", "person.pension_contribution", Period::Month,
                "pension contribution",
                "Employee share of statutory pension insurance (RV-Beitrag AN) this month"),
            employee_money("health_contribution", "person.health_insurance_contribution",
                Period::Month, "health insurance contribution",
                "Employee share of statutory health insurance including the additional contribution (KV-Beitrag AN) this month"),
            employee_money("care_contribution", "person.care_insurance_contribution",
                Period::Month, "care insurance contribution",
                "Employee share of long-term care insurance (PV-Beitrag AN) this month"),
            employee_money("unemployment_contribution", "person.unemployment_insurance_contribution",
                Period::Month, "unemployment insurance contribution",
                "Employee share of unemployment insurance (AV-Beitrag AN) this month"),
            slot("pay_month", None, Target::Subject, ValueKind::Period, PERIOD, "pay month",
                "The month this payslip covers (Abrechnungsmonat)"),
```

   Keep `gross` and `net` but build them with `employee_money` (same descriptions; `gross` stays `required`).

5. Lohnsteuerbescheinigung: after `gross_wage`, add (annual; descriptions name the official line and label):

```rust
            employee_money("wage_tax", "person.wage_tax", Period::Year, "wage tax",
                "Line 4: Einbehaltene Lohnsteuer von 3."),
            employee_money("solidarity_surcharge", "person.solidarity_surcharge", Period::Year,
                "solidarity surcharge", "Line 5: Einbehaltener Solidaritätszuschlag von 3."),
            employee_money("church_tax", "person.church_tax", Period::Year, "church tax",
                "Line 6: Einbehaltene Kirchensteuer des Arbeitnehmers von 3. (line 7 is the spouse's, not this)"),
            employee_money("pension_contribution", "person.pension_contribution", Period::Year,
                "pension contribution",
                "Line 23 a: Arbeitnehmeranteil zur gesetzlichen Rentenversicherung (line 22 is the employer's)"),
            employee_money("health_contribution", "person.health_insurance_contribution",
                Period::Year, "health insurance contribution",
                "Line 25: Arbeitnehmerbeiträge zur gesetzlichen Krankenversicherung"),
            employee_money("care_contribution", "person.care_insurance_contribution",
                Period::Year, "care insurance contribution",
                "Line 26: Arbeitnehmerbeiträge zur sozialen Pflegeversicherung"),
            employee_money("unemployment_contribution", "person.unemployment_insurance_contribution",
                Period::Year, "unemployment insurance contribution",
                "Line 27: Arbeitnehmerbeiträge zur Arbeitslosenversicherung"),
```

   and change `gross_wage` description to `"Line 3: Bruttoarbeitslohn einschl. Sachbezüge for the certificate period"`.

6. Tax assessment: before `DOCUMENT_DATE` add:

```rust
            slot("tax_year", None, Target::Subject, ValueKind::Period, PERIOD, "tax year",
                "The calendar year assessed (Veranlagungszeitraum)"),
            employee_money("taxable_income", "person.taxable_income", Period::Year,
                "taxable income", "Zu versteuerndes Einkommen for the tax year"),
            employee_money("income_tax_assessed", "person.income_tax_assessed", Period::Year,
                "assessed income tax",
                "Festgesetzte Einkommensteuer for the tax year, not amounts already paid or still due"),
            valid(
                slot("tax_balance", Some("person.tax_balance"), Target::Subject,
                    ValueKind::Balance, MONEY, "refund or back payment",
                    "The remaining amount of this assessment: Erstattung (refund) or Nachzahlung (payment due)"),
                SlotValidity::DocumentPeriod,
            ),
            valid(
                slot("payment_due", Some("person.tax_payment_due"), Target::Subject,
                    ValueKind::Date, DATE, "payment due date",
                    "The date by which a back payment must be paid, if printed"),
                SlotValidity::DocumentPeriod,
            ),
```

7. `crates/me-core/src/vocabulary.rs`: add rows in the same tuple format next to `person.net_income`:

```rust
    ("person.wage_tax", "Wage tax", "money", false, &[Person], &[]),
    ("person.solidarity_surcharge", "Solidarity surcharge", "money", false, &[Person], &[]),
    ("person.church_tax", "Church tax", "money", false, &[Person], &[]),
    ("person.pension_contribution", "Pension contribution", "money", false, &[Person], &[]),
    ("person.health_insurance_contribution", "Health insurance contribution", "money", false, &[Person], &[]),
    ("person.care_insurance_contribution", "Care insurance contribution", "money", false, &[Person], &[]),
    ("person.unemployment_insurance_contribution", "Unemployment insurance contribution", "money", false, &[Person], &[]),
    ("person.taxable_income", "Taxable income", "money", false, &[Person], &[]),
    ("person.income_tax_assessed", "Assessed income tax", "money", false, &[Person], &[]),
    ("person.tax_balance", "Tax refund or back payment", "money", false, &[Person], &[]),
    ("person.tax_payment_due", "Tax payment due", "date", false, &[Person], &[]),
    ("person.tax_class", "Tax class", "text", false, &[Person], &[]),
```

   (Match the file's existing multi-line formatting; `cargo fmt` will not reflow tuple literals, so write them like the neighbours.) `vocabulary::install` uses `INSERT OR IGNORE` on every unlock, so existing vaults receive the properties.

8. `crates/me-core/src/lib.rs`: extend the `pub use doc_types::{…}` list with `TAX_CLASSES, default_currency`.

- [ ] **Step 4: Run the tests**

Run: `./scripts/cargo test -p me-core --lib doc_types::tests`
Expected: PASS, including the existing `registry_is_internally_consistent_and_uses_the_vocabulary`.

Run: `./scripts/cargo test -p me-core && ./scripts/cargo test -p me-agent`
Expected: PASS. If a `graph_pipeline_tests` test counts payslip slot questions, update its expected count to the new slot list (those tests are deleted in Task 8).

- [ ] **Step 5: Commit**

```bash
./scripts/cargo fmt --all
git add crates/me-core/src/doc_types.rs crates/me-core/src/vocabulary.rs crates/me-core/src/lib.rs
git commit -m "Add tax values, periods and balances to the document registry

Payslips, wage tax certificates and tax assessments now list wage tax,
solidarity surcharge, church tax, tax class, social contributions,
taxable income, assessed tax and the signed refund or back payment.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Typing grounded reader values

**Files:**
- Create: `crates/me-core/src/typing.rs`
- Modify: `crates/me-core/src/lib.rs` (`mod typing; pub use typing::*;`)

**Interfaces:**
- Consumes: `find_candidates`, `parse_bare_amount`, `currencies_in`, `has_euro_cent_columns` (Task 1); `ValueKind`, `default_currency`, `TAX_CLASSES` (Task 2).
- Produces:

```rust
pub struct TypingContext { pub currency: Option<&'static str>, pub euro_cent: bool, pub year: i32 }
impl TypingContext { pub fn new(segments: &[SourceSegment<'_>], doc_type: Option<&str>, year: i32) -> Self }
pub fn document_currency(segments: &[SourceSegment<'_>]) -> Option<&'static str>
#[derive(Clone, Debug, PartialEq)]
pub struct Located { pub segment_id: String, pub start: usize, pub end: usize, pub quote_start: usize, pub quote_end: usize, pub line: String }
pub fn locate(segments: &[SourceSegment<'_>], segment_id: &str, quote: &str, value: &str) -> Option<Located>
pub fn type_value(ctx: &TypingContext, at: &Located, label: &str, value: &str, kind: ValueKind) -> Option<Candidate>
pub fn category_key(options: &'static [(&'static str, &'static str)], value: &str) -> Option<&'static str>
pub fn amount_of(ctx: &TypingContext, value: &str) -> Option<(String, Option<String>)> // (amount, currency)
```

- [ ] **Step 1: Write the failing tests** (bottom of `typing.rs`)

```rust
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
        let c = type_value(&ctx, &at, "Lohnsteuer", "1.032,58", ValueKind::Money(Some(Period::Month))).unwrap();
        assert_eq!(c.kind, K::Money);
        assert_eq!(c.value, V::Money { amount: "1032.58".into(), currency: "EUR".into() });
        assert_eq!(c.label.as_deref(), Some("Lohnsteuer"));
    }

    #[test]
    fn euro_cent_cells_and_whole_amounts_type_for_money_slots() {
        let segments = seg(LSTB);
        let ctx = TypingContext::new(&segments, Some("wage_tax_certificate"), 2026);
        assert!(ctx.euro_cent);
        let quote = "4. Einbehaltene Lohnsteuer von 3.   12.390   96";
        let at = locate(&segments, "s0", quote, "12.390   96").unwrap();
        let c = type_value(&ctx, &at, "4. Einbehaltene Lohnsteuer von 3.", "12.390   96", ValueKind::Money(Some(Period::Year))).unwrap();
        assert_eq!(c.value, V::Money { amount: "12390.96".into(), currency: "EUR".into() });
        let whole = seg("Festgesetzt: Einkommensteuer 4.200\n");
        let ctx = TypingContext::new(&whole, Some("tax_assessment"), 2026);
        let at = locate(&whole, "s0", "Einkommensteuer 4.200", "4.200").unwrap();
        let c = type_value(&ctx, &at, "Einkommensteuer", "4.200", ValueKind::Money(Some(Period::Year))).unwrap();
        assert_eq!(c.value, V::Money { amount: "4200".into(), currency: "EUR".into() });
    }

    #[test]
    fn values_that_cannot_hold_the_slot_kind_are_rejected() {
        let segments = seg(PAYSLIP);
        let ctx = TypingContext::new(&segments, Some("payslip"), 2026);
        let at = locate(&segments, "s0", "Krankenkasse   AOK Bayern", "AOK Bayern").unwrap();
        assert!(type_value(&ctx, &at, "Krankenkasse", "AOK Bayern", ValueKind::Money(None)).is_none());
        assert!(type_value(&ctx, &at, "Krankenkasse", "AOK Bayern", ValueKind::Date).is_none());
        let none = TypingContext { currency: None, euro_cent: false, year: 2026 };
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
        assert_eq!(category_key(crate::TAX_CLASSES, "Steuerklasse III"), Some("III"));
        assert_eq!(category_key(crate::TAX_CLASSES, "IV/Faktor"), Some("IV"));
        assert_eq!(category_key(crate::TAX_CLASSES, "7"), None);
    }

    #[test]
    fn periods_type_from_month_values() {
        let text = "Abrechnungsmonat Januar 2026\n";
        let segments = seg(text);
        let ctx = TypingContext::new(&segments, Some("payslip"), 2026);
        let at = locate(&segments, "s0", "Abrechnungsmonat Januar 2026", "Januar 2026").unwrap();
        let c = type_value(&ctx, &at, "Abrechnungsmonat", "Januar 2026", ValueKind::Period).unwrap();
        assert_eq!(c.value, V::Period { start: "2026-01-01".into(), end: "2026-01-31".into() });
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `./scripts/cargo test -p me-core --lib typing::tests`
Expected: FAIL to compile (module missing).

- [ ] **Step 3: Implement `typing.rs`**

```rust
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
            euro_cent: segments.iter().any(|s| candidates::has_euro_cent_columns(s.text)),
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
    (all.len() == 1).then(|| *all.iter().next().unwrap_or(&"EUR"))
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
    let line_start = text[..quote_start].rfind('\n').map_or(0, |i| i + 1);
    let quote_end = quote_start + quote.len();
    let line_end = text[quote_end..].find('\n').map_or(text.len(), |i| quote_end + i);
    Some(Located {
        segment_id: segment_id.to_owned(),
        start,
        end: start + value.len(),
        quote_start,
        quote_end,
        line: text[line_start..line_end].trim().chars().take(240).collect(),
    })
}

fn scan_value(value: &str, year: i32) -> Vec<Candidate> {
    find_candidates(&[SourceSegment { id: "v", text: value }], &[], year)
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
            (CandidateKind::Money, CandidateValue::Money { amount, currency: currency? }, false)
        }
        ValueKind::Date => {
            let c = whole(CandidateKind::Date)?;
            (CandidateKind::Date, c.value, false)
        }
        ValueKind::Period => {
            let c = whole(CandidateKind::Period)?;
            (CandidateKind::Period, c.value, false)
        }
        ValueKind::Identifier => {
            let checked = found.iter().find(|c| {
                c.checksum
                    && matches!(
                        c.kind,
                        CandidateKind::TaxId | CandidateKind::SocialInsuranceNumber | CandidateKind::Iban
                    )
            });
            match checked {
                Some(c) => (CandidateKind::Identifier, c.value.clone(), true),
                None => {
                    let text = value.trim();
                    if text.is_empty() {
                        return None;
                    }
                    (CandidateKind::Identifier, CandidateValue::Identifier(text.to_owned()), false)
                }
            }
        }
        ValueKind::Category(options) => {
            let key = category_key(options, value)?;
            (CandidateKind::LabelValue, CandidateValue::Text(key.to_owned()), false)
        }
        ValueKind::Text | ValueKind::Name => {
            let text = value.trim();
            if text.is_empty() {
                return None;
            }
            (CandidateKind::LabelValue, CandidateValue::Text(text.to_owned()), false)
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
        label: Some(label.chars().take(80).collect()),
        checksum,
    })
}
```

Add `mod typing; pub use typing::*;` to `lib.rs`, and make `candidates` reachable as `crate::candidates` (it is already `mod candidates;` in `lib.rs`; `pub(crate)` functions are visible).

- [ ] **Step 4: Run the tests**

Run: `./scripts/cargo test -p me-core --lib typing::tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
./scripts/cargo fmt --all
git add crates/me-core/src/typing.rs crates/me-core/src/lib.rs
git commit -m "Type grounded reader values locally

Code decides whether a quoted value is money in which currency, a date,
a period, a checksum-valid identifier or a category. The currency comes
from the document, then the type; it is never guessed.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Omission sweep

**Files:**
- Create: `crates/me-core/src/coverage.rs`
- Modify: `crates/me-core/src/lib.rs` (`mod coverage; pub use coverage::*;`)

**Interfaces:**
- Consumes: `Candidate` list from `find_candidates` over the full document; quote spans from `Located` (Task 3).
- Produces:

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Uncovered { pub segment_id: String, pub start: usize, pub end: usize, pub kind: CandidateKind, pub label: Option<String>, pub text: String, pub line: String }
pub fn worth_reading(c: &Candidate) -> bool
pub fn uncovered(candidates: &[Candidate], covered: &[(String, usize, usize)]) -> Vec<Uncovered>
```

- [ ] **Step 1: Write the failing test** (bottom of `coverage.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SourceSegment, find_candidates, locate};

    const PAYSLIP: &str = include_str!("../fixtures/documents/payslip_datev_rows.txt");

    #[test]
    fn values_no_fact_covers_are_listed_and_covered_ones_are_not() {
        let segments = [SourceSegment { id: "s0", text: PAYSLIP }];
        let found = find_candidates(&segments, &[], 2026);
        let span = |quote: &str| {
            let at = locate(&segments, "s0", quote, quote).unwrap();
            ("s0".to_owned(), at.quote_start, at.quote_end)
        };
        let covered = vec![span("Gesamt-Brutto   5.340,00"), span("Lohnsteuer   1.032,58")];
        let open = uncovered(&found, &covered);
        let texts: Vec<&str> = open.iter().map(|u| u.text.as_str()).collect();
        assert!(texts.contains(&"3.210,05"), "net pay must be listed: {texts:?}");
        assert!(texts.contains(&"435,21"));
        assert!(texts.contains(&"65929970489"), "checksum tax ID must be listed");
        assert!(!texts.contains(&"5.340,00") || open.iter().all(|u| u.label.as_deref() != Some("Gesamt-Brutto")));
        assert!(!open.iter().any(|u| u.label.as_deref() == Some("Lohnsteuer") && u.text == "1.032,58" && u.line.starts_with("Lohnsteuer")));
        // Unlabeled or free-text candidates are not audit material.
        assert!(open.iter().all(|u| u.label.is_some() || matches!(u.kind, CandidateKind::TaxId | CandidateKind::Iban | CandidateKind::SocialInsuranceNumber | CandidateKind::Mrz)));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `./scripts/cargo test -p me-core --lib coverage::tests`
Expected: FAIL to compile.

- [ ] **Step 3: Implement `coverage.rs`**

```rust
//! Omission sweep. The local scanners run over the whole document; every value
//! worth reading that no reader fact quotes is listed, audited once, and whatever
//! stays uncovered is stored as "not interpreted" instead of disappearing.
use crate::{Candidate, CandidateKind};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Uncovered {
    pub segment_id: String,
    pub start: usize,
    pub end: usize,
    pub kind: CandidateKind,
    pub label: Option<String>,
    pub text: String,
    pub line: String,
}

/// Checksum-validated identifiers, and amounts or dates that carry a printed label.
pub fn worth_reading(c: &Candidate) -> bool {
    use CandidateKind::*;
    match c.kind {
        Iban | TaxId | SocialInsuranceNumber | Mrz => c.checksum,
        Money | Amount | Date => c.label.is_some(),
        _ => false,
    }
}

/// Candidates worth reading whose span lies inside no covered quote span.
pub fn uncovered(candidates: &[Candidate], covered: &[(String, usize, usize)]) -> Vec<Uncovered> {
    candidates
        .iter()
        .filter(|c| worth_reading(c))
        .filter(|c| {
            !covered
                .iter()
                .any(|(seg, s, e)| *seg == c.segment_id && *s <= c.start && c.end <= *e)
        })
        .map(|c| Uncovered {
            segment_id: c.segment_id.clone(),
            start: c.start,
            end: c.end,
            kind: c.kind,
            label: c.label.clone(),
            text: c.text.clone(),
            line: c.line.clone(),
        })
        .collect()
}
```

Add `mod coverage; pub use coverage::*;` in `lib.rs`.

- [ ] **Step 4: Run the test**

Run: `./scripts/cargo test -p me-core --lib coverage::tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
./scripts/cargo fmt --all
git add crates/me-core/src/coverage.rs crates/me-core/src/lib.rs
git commit -m "List scanner-visible values that no reader fact covers

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Read store, migration 16 and Resolve v2

**Files:**
- Create: `crates/me-core/migrations/016_document_read.sql`
- Create: `crates/me-core/src/reading.rs`, `crates/me-core/src/reading_tests.rs`
- Modify: `crates/me-core/src/vault.rs` (create + upgrade to 16, range `1..=16`, version assertions)
- Modify: `crates/me-core/src/settings.rs:187-190` (test expects 16)
- Modify: `crates/me-core/src/graph.rs`
- Modify: `crates/me-core/src/lib.rs` (`mod reading; pub use reading::*;`)

**Interfaces:**
- Consumes: `DocumentGraph`, `resolve_document`, `Uncovered`.
- Produces (exact):

```rust
// graph.rs
pub const GRAPH_POLICY: &str = "import-graph-v2";
pub struct DocumentGraph { …, #[serde(default)] pub correction: bool }
pub struct GraphOutcome { pub entities_created: usize, pub assertions: usize, pub checks: usize,
    pub slot_assertions: BTreeMap<String, String>, pub recorded: BTreeSet<String> }

// reading.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum FactState { Verified, Uncertain, Unverified, Uninterpreted }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReadFact { pub label: String, pub value: String, pub segment_id: String, pub start: usize, pub end: usize,
    pub context: String, pub owner: String, pub owner_entity: Option<String>, pub owner_name: Option<String>,
    pub period: Option<String>, pub slot: Option<String>, pub state: FactState, pub confidence: Option<f64> }
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DocumentRead { pub run_id: String, pub graph: Option<DocumentGraph>, pub facts: Vec<ReadFact>,
    pub uninterpreted: Vec<Uncovered>, pub rejected: BTreeMap<String, usize> }
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ReadOutcome { pub values: usize, pub in_profile: usize, pub checks: usize, pub uninterpreted: usize }
impl Vault {
    pub fn apply_read(&mut self, source: &str, read: &DocumentRead) -> Result<ReadOutcome>;
    pub fn complete_read_run(&mut self, run_id: &str) -> Result<()>;
}
```

- [ ] **Step 1: Write the migration**

`crates/me-core/migrations/016_document_read.sql`:

```sql
-- Deep-first reading. Document facts are derived from encrypted originals and are
-- replaced on every re-read of their source; profile values stay assertions.
CREATE TABLE document_fact (
 id TEXT PRIMARY KEY,
 source_id TEXT NOT NULL REFERENCES source(id),
 run_id TEXT NOT NULL,
 ordinal INTEGER NOT NULL,
 label TEXT NOT NULL,
 value TEXT NOT NULL,
 context_quote TEXT NOT NULL,
 locator_json TEXT NOT NULL CHECK(json_valid(locator_json)),
 owner TEXT NOT NULL CHECK(owner IN ('self','household','party','organization','unclear','unknown')),
 owner_entity_id TEXT REFERENCES entity(id),
 owner_name TEXT,
 period_kind TEXT CHECK(period_kind IS NULL OR period_kind IN ('document','cumulative','other','none')),
 slot TEXT,
 assertion_id TEXT REFERENCES assertion(id),
 state TEXT NOT NULL CHECK(state IN ('verified','uncertain','unverified','uninterpreted')),
 confidence REAL CHECK(confidence IS NULL OR confidence BETWEEN 0 AND 1),
 recorded_at TEXT NOT NULL
) STRICT;
CREATE INDEX document_fact_source ON document_fact(source_id, state, ordinal);
-- Counts and rejection codes only; the Imports line and the self-check read this.
CREATE TABLE read_summary (
 source_id TEXT PRIMARY KEY REFERENCES source(id),
 run_id TEXT NOT NULL,
 policy TEXT NOT NULL,
 values_read INTEGER NOT NULL,
 in_profile INTEGER NOT NULL,
 checks INTEGER NOT NULL,
 uninterpreted INTEGER NOT NULL,
 rejected_json TEXT NOT NULL CHECK(json_valid(rejected_json)),
 recorded_at TEXT NOT NULL
) STRICT;
-- Documents finished by the limited tiered reader are read again behind new imports.
UPDATE document_evaluation
 SET state='queued', priority=0, warning_message=NULL, error_message=NULL
 WHERE state='done'
 AND source_id IN (SELECT source_id FROM document_profile)
 AND NOT EXISTS(SELECT 1 FROM job j WHERE j.source_id=document_evaluation.source_id AND j.kind='extract_facts' AND j.state IN ('done','needs_review'));
PRAGMA user_version = 16;
```

In `vault.rs` `create_with_keys` add `tx.execute_batch(include_str!("../migrations/016_document_read.sql"))?;` after 015. In the unlock path change `1..=15` to `1..=16` and add after the `version < 15` block:

```rust
        if version < 16 {
            let tx = db.transaction()?;
            tx.execute_batch(include_str!("../migrations/016_document_read.sql"))?;
            tx.commit()?;
        }
```

Update `assert_eq!(version, 15)` in `vault.rs` tests and the settings test to `16`.

- [ ] **Step 2: Write the failing tests** (`crates/me-core/src/reading_tests.rs`, wired with `#[cfg(test)] #[path = "reading_tests.rs"] mod tests;` at the bottom of `reading.rs`)

The tests use a synthetic vault and a synthetic source. Reuse the helper that `graph_tests.rs` uses to create a personal document source (read `crates/me-core/src/graph_tests.rs` for `fn vault_with_source` or equivalent and copy it here; it must insert a `source` with `sensitivity='personal'`, `retention='keep'` and a `collection_item`).

```rust
use super::*;
use crate::{Candidate, CandidateKind, CandidateValue, ConfidenceSource, DocumentGraph, SlotContent, SlotValue};

fn money(slot: &str, amount: &str, start: usize) -> SlotValue {
    SlotValue {
        slot: slot.into(),
        content: SlotContent::Candidate(Box::new(Candidate {
            id: String::new(),
            kind: CandidateKind::Money,
            text: amount.into(),
            value: CandidateValue::Money { amount: amount.into(), currency: "EUR".into() },
            segment_id: "seg".into(),
            start,
            end: start + amount.len(),
            line: format!("Betrag {amount}"),
            label: Some("Betrag".into()),
            checksum: false,
        })),
        period: None,
        confidence: 0.95,
        source: ConfidenceSource::Typesafe,
        value_checked: false,
        check: false,
    }
}
fn month(start: &str, end: &str) -> SlotValue {
    SlotValue {
        slot: "pay_month".into(),
        content: SlotContent::Candidate(Box::new(Candidate {
            id: String::new(),
            kind: CandidateKind::Period,
            text: "Januar".into(),
            value: CandidateValue::Period { start: start.into(), end: end.into() },
            segment_id: "seg".into(),
            start: 0,
            end: 6,
            line: "Januar".into(),
            label: None,
            checksum: false,
        })),
        period: None,
        confidence: 0.95,
        source: ConfidenceSource::Typesafe,
        value_checked: false,
        check: false,
    }
}
fn payslip(subject: &str, values: Vec<SlotValue>) -> DocumentGraph {
    DocumentGraph {
        doc_type: "payslip".into(),
        subject: Some(subject.into()),
        subject_confidence: 0.95,
        values,
        models: vec![],
        correction: false,
    }
}
fn fact(label: &str, value: &str, slot: Option<&str>) -> ReadFact {
    ReadFact {
        label: label.into(),
        value: value.into(),
        segment_id: "seg".into(),
        start: 0,
        end: value.len(),
        context: String::new(),
        owner: "self".into(),
        owner_entity: None,
        owner_name: None,
        period: Some("document".into()),
        slot: slot.map(str::to_owned),
        state: FactState::Verified,
        confidence: Some(0.95),
    }
}

#[test]
fn a_read_stores_profile_values_document_facts_and_uninterpreted_values() {
    let (mut vault, source) = synthetic_payslip_source();
    let me = vault.profile_entity_id().unwrap();
    let read = DocumentRead {
        run_id: "run-1".into(),
        graph: Some(payslip(&me, vec![money("wage_tax", "1032.58", 10), month("2026-01-01", "2026-01-31")])),
        facts: vec![fact("Lohnsteuer", "1.032,58", Some("wage_tax")), fact("Kostenstelle", "4711", None)],
        uninterpreted: vec![crate::Uncovered {
            segment_id: "seg".into(), start: 40, end: 46, kind: CandidateKind::Amount,
            label: Some("KV-Beitrag".into()), text: "435,21".into(), line: "KV-Beitrag 435,21".into(),
        }],
        rejected: [("value_not_in_quote".to_owned(), 1)].into(),
    };
    let outcome = vault.apply_read(&source, &read).unwrap();
    assert_eq!(outcome, ReadOutcome { values: 2, in_profile: 1, checks: 0, uninterpreted: 1 });
    let linked: i64 = vault.db.query_row("SELECT count(*) FROM document_fact WHERE source_id=? AND assertion_id IS NOT NULL", [&source], |r| r.get(0)).unwrap();
    assert_eq!(linked, 1);
    let (from, to): (String, Option<String>) = vault.db.query_row(
        "SELECT valid_from,valid_to FROM assertion_state WHERE property_key='person.wage_tax' AND state='accept'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!((from.as_str(), to.as_deref()), ("2026-01-01", Some("2026-02-01")));
}

#[test]
fn twelve_monthly_payslips_form_a_timeline_without_conflicts() {
    let (mut vault, _) = synthetic_payslip_source();
    let me = vault.profile_entity_id().unwrap();
    for m in 1..=12u32 {
        let source = add_synthetic_source(&mut vault, &format!("payslip-{m}"));
        let last = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][m as usize - 1];
        let amount = format!("{}.00", 1000 + m);
        let read = DocumentRead {
            run_id: format!("run-{m}"),
            graph: Some(payslip(&me, vec![money("wage_tax", &amount, 10), month(&format!("2026-{m:02}-01"), &format!("2026-{m:02}-{last}"))])),
            ..Default::default()
        };
        vault.apply_read(&source, &read).unwrap();
    }
    let (count, conflicts): (i64, i64) = vault.db.query_row(
        "SELECT count(*),sum(r.check_reason='conflict') FROM assertion_state a JOIN assertion_review r ON r.assertion_id=a.id WHERE a.property_key='person.wage_tax' AND a.state='accept'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!((count, conflicts), (12, 0));
}

#[test]
fn monthly_and_annual_values_never_conflict() {
    let (mut vault, source) = synthetic_payslip_source();
    let me = vault.profile_entity_id().unwrap();
    let read = DocumentRead { run_id: "m".into(), graph: Some(payslip(&me, vec![money("wage_tax", "1032.58", 10), month("2026-01-01", "2026-01-31")])), ..Default::default() };
    vault.apply_read(&source, &read).unwrap();
    let certificate = add_synthetic_source(&mut vault, "lstb");
    let mut annual = payslip(&me, vec![money("wage_tax", "12390.96", 10)]);
    annual.doc_type = "wage_tax_certificate".into();
    annual.values.push(SlotValue { slot: "period_start".into(), ..date_value("2026-01-01") });
    annual.values.push(SlotValue { slot: "period_end".into(), ..date_value("2026-12-31") });
    vault.apply_read(&certificate, &DocumentRead { run_id: "y".into(), graph: Some(annual), ..Default::default() }).unwrap();
    let conflicts: i64 = vault.db.query_row("SELECT count(*) FROM assertion_review WHERE check_reason='conflict'", [], |r| r.get(0)).unwrap();
    assert_eq!(conflicts, 0);
}

#[test]
fn a_reread_retracts_automatic_values_it_no_longer_finds_but_keeps_user_decisions() {
    let (mut vault, source) = synthetic_payslip_source();
    let me = vault.profile_entity_id().unwrap();
    let first = DocumentRead { run_id: "a".into(), graph: Some(payslip(&me, vec![money("wage_tax", "1032.58", 10), money("church_tax", "10.00", 30), month("2026-01-01", "2026-01-31")])), ..Default::default() };
    vault.apply_read(&source, &first).unwrap();
    let church: String = vault.db.query_row("SELECT id FROM assertion_state WHERE property_key='person.church_tax' AND state='accept'", [], |r| r.get(0)).unwrap();
    vault.confirm_assertion(&church).unwrap();
    let second = DocumentRead { run_id: "b".into(), graph: Some(payslip(&me, vec![month("2026-01-01", "2026-01-31")])), ..Default::default() };
    vault.apply_read(&source, &second).unwrap();
    let wage: i64 = vault.db.query_row("SELECT count(*) FROM assertion_state WHERE property_key='person.wage_tax' AND state='accept'", [], |r| r.get(0)).unwrap();
    let kept: i64 = vault.db.query_row("SELECT count(*) FROM assertion_state WHERE property_key='person.church_tax' AND state='accept'", [], |r| r.get(0)).unwrap();
    assert_eq!((wage, kept), (0, 1));
}

#[test]
fn a_correction_supersedes_the_earlier_value_for_the_same_period() {
    let (mut vault, source) = synthetic_payslip_source();
    let me = vault.profile_entity_id().unwrap();
    vault.apply_read(&source, &DocumentRead { run_id: "o".into(), graph: Some(payslip(&me, vec![money("wage_tax", "1032.58", 10), month("2026-01-01", "2026-01-31")])), ..Default::default() }).unwrap();
    let fixed = add_synthetic_source(&mut vault, "correction");
    let mut graph = payslip(&me, vec![money("wage_tax", "1040.00", 10), month("2026-01-01", "2026-01-31")]);
    graph.correction = true;
    vault.apply_read(&fixed, &DocumentRead { run_id: "c".into(), graph: Some(graph), ..Default::default() }).unwrap();
    let values: Vec<String> = vault.db.prepare("SELECT value_json FROM assertion_state WHERE property_key='person.wage_tax' AND state='accept'").unwrap()
        .query_map([], |r| r.get(0)).unwrap().collect::<std::result::Result<_, _>>().unwrap();
    assert_eq!(values.len(), 1);
    assert!(values[0].contains("1040.00"));
    let conflicts: i64 = vault.db.query_row("SELECT count(*) FROM assertion_review WHERE check_reason='conflict'", [], |r| r.get(0)).unwrap();
    assert_eq!(conflicts, 0);
}

#[test]
fn migration_sixteen_requeues_documents_read_by_the_tiered_reader() {
    let (vault, source, root) = synthetic_vault_on_disk();
    vault.db.execute_batch(&format!(
        "INSERT INTO document_profile(source_id,family,doc_type,family_confidence,type_confidence,tier,graph_state,classified_at) VALUES('{source}','employment','payslip',0.9,0.9,'eager','resolved','2026-09-27');
         UPDATE document_evaluation SET state='done' WHERE source_id='{source}';
         DROP TABLE document_fact; DROP TABLE read_summary; PRAGMA user_version=15;")).unwrap();
    crate::revisions::remove_for_legacy_fixture(&vault.db);
    drop(vault);
    let vault = crate::Vault::unlock(&root, SYNTHETIC_PASSWORD).unwrap();
    let (state, priority): (String, i64) = vault.db.query_row("SELECT state,priority FROM document_evaluation WHERE source_id=?", [&source], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!((state.as_str(), priority), ("queued", 0));
}
```

Write the fixture helpers `synthetic_payslip_source`, `add_synthetic_source`, `synthetic_vault_on_disk`, `date_value` and `SYNTHETIC_PASSWORD` in the same test file by copying the source-creation SQL used in `graph_tests.rs` (source + collection_item + document_evaluation rows, `sensitivity='personal'`, `retention='keep'`). `date_value(d)` returns a `SlotValue` with a `CandidateValue::Date(d)` candidate.

- [ ] **Step 3: Run to verify failure**

Run: `./scripts/cargo test -p me-core --lib reading::tests`
Expected: FAIL to compile (`apply_read`, `correction`).

- [ ] **Step 4: Resolve v2 in `graph.rs`**

1. `pub const GRAPH_POLICY: &str = "import-graph-v2";`
2. `DocumentGraph`: add `#[serde(default)] pub correction: bool,`.
3. `GraphOutcome`: derive stays; add `pub slot_assertions: BTreeMap<String, String>, pub recorded: BTreeSet<String>,` (remove `Eq`/`PartialEq` only if they no longer derive; `BTreeMap<String,String>` supports both).
4. `Resolved::validity`, `DocumentPeriod` arm:

```rust
            SlotValidity::DocumentPeriod => {
                match (self.date("period_start"), self.date("period_end")) {
                    (Some(from), Some(end)) if end >= from => Validity::Interval { to: next_day(&end), from },
                    _ => match self.period() {
                        Some((from, end)) => Validity::Interval { to: next_day(&end), from },
                        None => Validity::Unknown,
                    },
                }
            }
```

   with

```rust
    /// The first period-typed slot (pay month, tax year), as inclusive dates.
    fn period(&self) -> Option<(String, String)> {
        self.kind.slots.iter().filter(|s| s.value == ValueKind::Period).find_map(|s| {
            match &self.values.get(s.key)?.content {
                SlotContent::Candidate(c) => match &c.value {
                    CandidateValue::Period { start, end } => Some((start.clone(), end.clone())),
                    _ => None,
                },
                SlotContent::Category { .. } => None,
            }
        })
    }
```

5. `record` closure: return `Result<Option<String>>` (the assertion id when recorded) and insert it into `outcome.recorded`. In the slot loop, after `record(...)`, `if let Some(a) = recorded { outcome.slot_assertions.insert(slot.key.to_owned(), a); }`. Constants and links ignore the returned id except for `outcome.recorded`.
6. Period-aware conflicts in `mark_conflicts`: before `if a != b && va != vb && overlap(ta, tb)`, skip pairs whose money periods differ:

```rust
fn money_period(canonical: &str) -> Option<Value> {
    serde_json::from_str::<Value>(canonical).ok()?.get("period").cloned()
}
```

   and use `if a != b && va != vb && money_period(va) == money_period(vb) && overlap(ta, tb)`.
7. Corrections: add

```rust
/// A correcting document replaces earlier automatic values of the same property
/// and exact period from other sources, instead of conflicting with them.
fn supersede_corrected(
    tx: &Transaction<'_>,
    source: &str,
    subject: &str,
    property: &str,
    assertions: &BTreeSet<String>,
) -> Result<()> {
    for assertion in assertions {
        let (from, to): (Option<String>, Option<String>) = tx.query_row(
            "SELECT valid_from,valid_to FROM assertion WHERE id=?",
            [assertion],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let older = tx
            .prepare("SELECT a.id FROM assertion_state a WHERE a.subject_id=? AND a.property_key=? AND a.state='accept' AND a.id<>? AND a.valid_from IS ? AND a.valid_to IS ? AND NOT EXISTS(SELECT 1 FROM decision d WHERE d.assertion_id=a.id AND d.actor='user') AND NOT EXISTS(SELECT 1 FROM assertion_evidence e WHERE e.assertion_id=a.id AND e.source_id=?)")?
            .query_map(params![subject, property, assertion, from, to, source], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for old in older {
            policy_decision(tx, &old, "retract", "corrected_by_newer_document")?;
            tx.execute(
                "UPDATE collection_item SET deleted_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE current_assertion_id=? AND deleted_at IS NULL",
                [&old],
            )?;
        }
    }
    Ok(())
}
```

   In the `for ((subject, property), assertions) in touched` loop call `if graph.correction { supersede_corrected(tx, source, &subject, &property, &assertions)?; }` before `retimeline`.

- [ ] **Step 5: Implement `reading.rs`**

```rust
//! The read store: profile values go through Resolve; every other grounded value
//! is a document fact of its source; values nobody interpreted stay visible.
use crate::{
    DocumentGraph, Result, Uncovered, Vault,
    graph::{GRAPH_POLICY, resolve_document},
    vault::id,
};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactState {
    Verified,
    Uncertain,
    Unverified,
    Uninterpreted,
}
impl FactState {
    pub fn key(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Uncertain => "uncertain",
            Self::Unverified => "unverified",
            Self::Uninterpreted => "uninterpreted",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReadFact {
    pub label: String,
    /// Raw printed value.
    pub value: String,
    pub segment_id: String,
    pub start: usize,
    pub end: usize,
    pub context: String,
    /// `self`, `household`, `party`, `organization`, `unclear` or `unknown`.
    pub owner: String,
    pub owner_entity: Option<String>,
    pub owner_name: Option<String>,
    /// `document`, `cumulative`, `other` or `none` for amounts.
    pub period: Option<String>,
    /// Registry slot this fact fills in the profile, if any.
    pub slot: Option<String>,
    pub state: FactState,
    pub confidence: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DocumentRead {
    pub run_id: String,
    pub graph: Option<DocumentGraph>,
    pub facts: Vec<ReadFact>,
    pub uninterpreted: Vec<Uncovered>,
    /// Grounding rejections by code (counts only).
    pub rejected: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ReadOutcome {
    pub values: usize,
    pub in_profile: usize,
    pub checks: usize,
    pub uninterpreted: usize,
}

fn locator(source: &str, segment: &str, start: usize, end: usize) -> String {
    json!({"kind":"candidate","source_id":source,"segment_id":segment,"start":start,"end":end}).to_string()
}

impl Vault {
    /// Replaces this source's read: document facts, automatic profile values and
    /// the summary. User decisions are never overridden.
    pub fn apply_read(&mut self, source: &str, read: &DocumentRead) -> Result<ReadOutcome> {
        let tx = self.db.transaction()?;
        let personal: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM source WHERE id=? AND sensitivity='personal' AND retention='keep')",
            [source],
            |r| r.get(0),
        )?;
        if !personal {
            return Err(crate::Error::Validation("This document is not available for analysis."));
        }
        let previous: BTreeSet<String> = tx
            .prepare("SELECT DISTINCT a.id FROM assertion_state a JOIN assertion_evidence e ON e.assertion_id=a.id WHERE e.source_id=? AND a.origin='extraction' AND a.state='accept' AND (SELECT actor FROM decision d WHERE d.assertion_id=a.id ORDER BY d.local_seq DESC LIMIT 1)='policy'")?
            .query_map([source], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<_, _>>()?;
        let graph = match &read.graph {
            Some(g) => Some(resolve_document(&tx, source, g)?),
            None => None,
        };
        let recorded = graph.as_ref().map(|g| g.recorded.clone()).unwrap_or_default();
        for stale in previous.difference(&recorded) {
            tx.execute("INSERT INTO decision(id,assertion_id,action,actor,policy_version,reason_code,recorded_at) VALUES(?,?,'retract','policy',?,'not_found_on_reread',strftime('%Y-%m-%dT%H:%M:%fZ','now'))", params![id(), stale, GRAPH_POLICY])?;
            tx.execute("UPDATE collection_item SET deleted_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE current_assertion_id=? AND kind='note' AND deleted_at IS NULL", [stale])?;
        }
        tx.execute("DELETE FROM document_fact WHERE source_id=?", [source])?;
        let slots = graph.as_ref().map(|g| g.slot_assertions.clone()).unwrap_or_default();
        let mut in_profile = 0;
        for (ordinal, f) in read.facts.iter().enumerate() {
            let assertion = f.slot.as_ref().and_then(|s| slots.get(s)).cloned();
            in_profile += usize::from(assertion.is_some());
            tx.execute(
                "INSERT INTO document_fact VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
                params![id(), source, read.run_id, ordinal as i64, f.label, f.value, f.context,
                    locator(source, &f.segment_id, f.start, f.end), f.owner, f.owner_entity, f.owner_name,
                    f.period, f.slot, assertion, f.state.key(), f.confidence.map(|c| c.clamp(0., 1.))],
            )?;
        }
        let base = read.facts.len();
        for (i, u) in read.uninterpreted.iter().enumerate() {
            tx.execute(
                "INSERT INTO document_fact VALUES(?,?,?,?,?,?,'',?,'unknown',NULL,NULL,NULL,NULL,NULL,'uninterpreted',NULL,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
                params![id(), source, read.run_id, (base + i) as i64, u.label.clone().unwrap_or_default(), u.text, locator(source, &u.segment_id, u.start, u.end)],
            )?;
        }
        let checks = graph.as_ref().map_or(0, |g| g.checks);
        let outcome = ReadOutcome { values: read.facts.len(), in_profile, checks, uninterpreted: read.uninterpreted.len() };
        tx.execute(
            "INSERT INTO read_summary VALUES(?,?,?,?,?,?,?,?,strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(source_id) DO UPDATE SET run_id=excluded.run_id,policy=excluded.policy,values_read=excluded.values_read,in_profile=excluded.in_profile,checks=excluded.checks,uninterpreted=excluded.uninterpreted,rejected_json=excluded.rejected_json,recorded_at=excluded.recorded_at",
            params![source, read.run_id, GRAPH_POLICY, outcome.values as i64, outcome.in_profile as i64, outcome.checks as i64, outcome.uninterpreted as i64, serde_json::to_string(&read.rejected).map_err(|_| crate::Error::Format)?],
        )?;
        tx.commit()?;
        Ok(outcome)
    }

    /// Marks a read's extraction run and job finished.
    pub fn complete_read_run(&mut self, run_id: &str) -> Result<()> {
        let tx = self.db.transaction()?;
        tx.execute("UPDATE extraction_run SET status='succeeded' WHERE id=? AND status='running'", [run_id])?;
        tx.execute("UPDATE job SET state='done',last_error_code=NULL WHERE kind='extract_facts' AND source_id=(SELECT source_id FROM extraction_run WHERE id=?)", [run_id])?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "reading_tests.rs"]
mod tests;
```

   `resolve_document` is `pub(crate)` already. `id()` is `vault::id` (`pub(crate)`). Add `mod reading; pub use reading::*;` to `lib.rs`.

   The re-read of the *same* source can re-record an identical assertion: `insert_assertion` dedups by semantic key and returns the existing id, so it stays in `recorded` and is not retracted.

- [ ] **Step 6: Run the tests**

Run: `./scripts/cargo test -p me-core`
Expected: PASS (new reading tests, graph tests, migration tests at version 16). If an existing graph test asserted `GRAPH_POLICY == "import-graph-v1"` or counted conflicts between monthly and annual values, update it to the new rules.

- [ ] **Step 7: Commit**

```bash
./scripts/cargo fmt --all
git add crates/me-core
git commit -m "Store reads with document facts and resolve periods and corrections

Migration 16 adds document facts and read summaries and re-queues
documents finished by the tiered reader. Resolve falls back to pay month
or tax year for validity, compares money only within one payment period,
lets corrections supersede earlier values and retracts automatic values a
re-read no longer finds.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Reading guides and the guided reader

**Files:**
- Create: `crates/me-agent/src/guides.rs`
- Modify: `crates/me-agent/src/lib.rs` (`pub mod guides;`)
- Modify: `crates/me-core/src/knowledge.rs` (`ExtractedFact.slot`), `crates/me-core/src/grounding.rs` (merge keeps slot), every `ExtractedFact { … }` literal (add `slot: String::new()`): `crates/me-core/tests/{checkpoints,documents,complete_import,knowledge_map,import_recovery,review_questions,knowledge}.rs`, `crates/me-core/examples/lean_ui_fixture.rs`, `crates/me-core/src/{domain_tests,knowledge,grounding}.rs`, `apps/desktop/examples/{knowledge_gallery,motion_gallery}.rs`
- Modify: `crates/me-agent/src/codex.rs`, `codex_pipeline.rs`, `codex_document.rs`, `typesafe_questions.rs` (`profile_questions`), `codex_batch_tests.rs`

**Interfaces:**
- Produces:

```rust
// me-agent guides.rs
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ChecklistItem { pub slot: String, pub label: String, pub description: String }
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReadingGuide { pub key: String, pub text: String, pub checklist: Vec<ChecklistItem> }
impl ReadingGuide { pub fn general() -> Self; pub fn slot_keys(&self) -> Vec<String> }
pub fn reading_guide(family: &str, doc_type: Option<&str>) -> ReadingGuide
// me-agent codex.rs
pub const PIPELINE: &str = "document-v6-guided-v1";
pub fn extract_document(home: &Path, input: &ExtractionInput, guide: &ReadingGuide, cancel: Arc<AtomicBool>, progress: impl FnMut(Progress), checkpoints: &mut impl Checkpoints) -> Result<ExtractionReport>
pub fn audit_document(home: &Path, input: &ExtractionInput, guide: &ReadingGuide, previous: &ExtractionOutput, uncovered: &[me_core::Uncovered], cancel: Arc<AtomicBool>, progress: impl FnMut(Progress), checkpoints: &mut impl Checkpoints) -> Result<ExtractionReport>
// me-core
pub struct ExtractedFact { …, #[serde(default)] pub slot: String }
```

- [ ] **Step 1: Write the failing tests** (bottom of `guides.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_payslip_gets_the_payroll_guide_and_its_checklist() {
        let g = reading_guide("employment", Some("payslip"));
        assert!(g.text.contains("Jahreswerte"));
        assert!(g.text.contains("Netto-Verdienst and Auszahlungsbetrag"));
        let keys = g.slot_keys();
        assert!(keys.contains(&"wage_tax".to_owned()) && keys.contains(&"gross".to_owned()));
        assert!(!keys.contains(&"period_start".to_owned()) || keys.contains(&"pay_month".to_owned()));
        assert_ne!(g.key, reading_guide("employment", None).key);
    }

    #[test]
    fn a_wage_tax_certificate_gets_the_official_line_semantics() {
        let g = reading_guide("tax", Some("wage_tax_certificate"));
        for needle in ["line 23", "line 22", "EUR and Ct", "Korrektur", "unbesetzt"] {
            assert!(g.text.contains(needle), "missing {needle}");
        }
    }

    #[test]
    fn unknown_or_lazy_types_are_read_with_family_or_general_rules() {
        let letter = reading_guide("correspondence", Some("letter"));
        assert!(letter.checklist.is_empty());
        assert!(letter.text.contains("sender"));
        let other = reading_guide("other", None);
        assert_eq!(other, ReadingGuide::general());
    }
}
```

And in `crates/me-agent/src/codex_batch_tests.rs` add:

```rust
#[test]
fn the_extraction_schema_offers_only_checklist_slots() {
    let guide = crate::guides::reading_guide("employment", Some("payslip"));
    let schema = crate::codex::guided_schema(&guide, &["s1".to_owned()]);
    let slot = &schema["properties"]["facts"]["items"]["properties"]["slot"]["enum"];
    let slots: Vec<&str> = slot.as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert!(slots.contains(&"wage_tax") && slots.contains(&"none"));
    assert!(!slots.contains(&"iban"));
    let required = schema["properties"]["facts"]["items"]["required"].as_array().unwrap();
    assert!(required.iter().any(|v| v == "slot"));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `./scripts/cargo test -p me-agent guides`
Expected: FAIL to compile.

- [ ] **Step 3: Add `slot` to `ExtractedFact`**

In `knowledge.rs`:

```rust
    /// Registry slot the reader proposes for this fact, or `none`/empty.
    #[serde(default)]
    pub slot: String,
```

Add `slot: String::new(),` to every `ExtractedFact { … }` literal listed in **Files** (find them with `grep -rn "ExtractedFact {" crates apps`). In `grounding.rs` `merge_extracted_fact`, keep a slot the duplicate carries:

```rust
    if let Some(old) = facts.iter_mut().find(|old| same_fact(old, &fact)) {
        if (old.slot.is_empty() || old.slot == "none") && !fact.slot.is_empty() {
            old.slot = fact.slot;
        }
        return;
    }
```

- [ ] **Step 4: Implement `guides.rs`**

Move `PAYROLL_GUIDE` and `CORRESPONDENCE_GUIDE` text from `codex_document.rs` into `guides.rs` (keep them verbatim, then append the additions below). Complete file:

```rust
//! Reading guides: meanings of document families and official forms, plus the
//! registry checklist of values that belong in the profile. Guides supply
//! meanings, never values. Private data never enters a lookup.
use me_core::doc_types;
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ChecklistItem {
    pub slot: String,
    pub label: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReadingGuide {
    /// Cache discriminator: family, type and registry version.
    pub key: String,
    pub text: String,
    pub checklist: Vec<ChecklistItem>,
}

impl ReadingGuide {
    pub fn general() -> Self {
        Self {
            key: format!("general|{}", doc_types::REGISTRY_VERSION),
            text: GENERAL.to_owned(),
            checklist: Vec::new(),
        }
    }
    pub fn slot_keys(&self) -> Vec<String> {
        self.checklist.iter().map(|c| c.slot.clone()).collect()
    }
}

const GENERAL: &str = "Keep document labels, printed periods and source layout. Do not assume a document type or country.";

pub(crate) const PAYROLL_GUIDE: &str = r#"<verbatim PAYROLL_GUIDE text from codex_document.rs>
Steuer-Brutto and SV-Brutto are not Gesamt-Brutto. A Nachberechnung or Korrektur month is its own period. Tag each monthly value with the checklist slot only when it is this payslip's own month; Jahreswerte, kumulierte Werte and Verdienstbescheinigung totals are slot none."#;

pub(crate) const CORRESPONDENCE_GUIDE: &str = r#"<verbatim CORRESPONDENCE_GUIDE text from codex_document.rs>"#;

const WAGE_TAX_CERTIFICATE_GUIDE: &str = r#"
Lohnsteuerbescheinigung (printout of the electronic wage tax certificate, BMF template). Rely on the printed labels; line numbers are hints and change between years.
Line 1 Bescheinigungszeitraum is the certificate period. Line 2 counts periods without wages (Anzahl "U") and letters S, M, F, FR.
Line 3 Bruttoarbeitslohn einschl. Sachbezüge. Line 4 Einbehaltene Lohnsteuer von 3. Line 5 Einbehaltener Solidaritätszuschlag von 3. Line 6 Kirchensteuer des Arbeitnehmers. Line 7 is the spouse's or partner's church tax: never the employee's.
Line 22 a/b are the employer's pension shares; line 23 a/b are the employee's shares (23 a statutory pension). Line 24 a–c are tax-free employer subsidies. Line 25 employee statutory health insurance, line 26 employee care insurance, line 27 employee unemployment insurance. From 2026 line 28 is unbesetzt; earlier years used it for private insurance amounts.
Amounts are printed in two cells, EUR and Ct. Return the value as the exact printed text spanning both cells of one row (for example "64.080   00") and quote the whole row. Never add, round or combine rows.
A certificate marked Korrektur replaces an earlier certificate for the same employer and period; Stornierung cancels it. Record these markers as facts.
Header fields: Identifikationsnummer (person.tax_id), Personalnummer, Geburtsdatum, Steuerklasse/Faktor, Kinderfreibeträge, Kirchensteuermerkmale, employer address and Steuernummer, and the Finanzamt the tax was paid to.
"#;

const TAX_ASSESSMENT_GUIDE: &str = r#"
Steuerbescheid. The Veranlagungszeitraum is the tax year. Distinguish festgesetzt (assessed), anzurechnen or bereits gezahlt (already paid or withheld) and verbleibend or abzurechnen (remaining). The remaining amount is either an Erstattung (refund to the taxpayer) or a Nachzahlung (payment due); keep the printed amount unsigned and keep its printed direction words in the quote. Vorauszahlungen for later years are future instalments, not this year's tax. Zu versteuerndes Einkommen is the taxable income. A Bescheid may change an earlier Bescheid (geändert nach § …); record that marker.
"#;

const IDENTITY_GUIDE: &str = "Identity document: distinguish printed fields from the machine-readable zone, the issue date from the expiry date, and the issuing authority from the holder.";
const BANKING_GUIDE: &str = "Bank document: distinguish account balance from individual transactions, debit from credit, and the account holder's IBAN from counterparties' IBANs.";
const INVOICE_GUIDE: &str = "Invoice: distinguish net amount, VAT and gross total, the invoice date from the due date, and the payee's bank details (they belong to the payee, not the recipient).";
const CONTRACT_GUIDE: &str = "Contract, insurance or housing document: distinguish start date, end date and notice period without computing dates; the premium or rent and its payment period; insured person, policyholder and payee.";

fn family_text(family: &str) -> &'static str {
    match family {
        "employment" => PAYROLL_GUIDE,
        "identity" => IDENTITY_GUIDE,
        "banking" => BANKING_GUIDE,
        "invoice" => INVOICE_GUIDE,
        "insurance" | "housing" | "contract" | "vehicle" => CONTRACT_GUIDE,
        "correspondence" | "health" => "",
        _ => GENERAL,
    }
}

fn type_text(doc_type: &str) -> &'static str {
    match doc_type {
        "wage_tax_certificate" => WAGE_TAX_CERTIFICATE_GUIDE,
        "tax_assessment" => TAX_ASSESSMENT_GUIDE,
        _ => "",
    }
}

/// Guide and checklist for a classified document. An uncertain type keeps only
/// its family's rules; the checklist comes only from an eager registry type.
pub fn reading_guide(family: &str, doc_type: Option<&str>) -> ReadingGuide {
    if family == "other" && doc_type.is_none() {
        return ReadingGuide::general();
    }
    let kind = doc_type.and_then(doc_types::doc_type);
    let mut text = String::new();
    for part in [family_text(family), doc_type.map_or("", type_text), CORRESPONDENCE_GUIDE] {
        if !part.is_empty() {
            text.push_str(part);
            text.push('\n');
        }
    }
    let checklist = kind
        .map(|k| {
            k.slots
                .iter()
                .filter(|s| s.mrz.is_none())
                .map(|s| ChecklistItem {
                    slot: s.key.to_owned(),
                    label: s.label.to_owned(),
                    description: s.description.to_owned(),
                })
                .collect()
        })
        .unwrap_or_default();
    ReadingGuide {
        key: format!("{family}|{}|{}", doc_type.unwrap_or("-"), doc_types::REGISTRY_VERSION),
        text,
        checklist,
    }
}
```

Replace the two `<verbatim …>` markers by moving the exact existing constant bodies from `codex_document.rs` (cut, not copy). Add `pub mod guides;` to `crates/me-agent/src/lib.rs`.

- [ ] **Step 5: Guide the reader**

`codex_document.rs` `instructions` becomes:

```rust
pub(super) fn instructions(profile: &Value, guide: &crate::guides::ReadingGuide) -> String {
    let layout = if profile["tabular"]["noul"].as_f64().is_some_and(|p| p >= 0.2) {
        "Potential table layout: preserve row and column associations, and avoid assigning a nearby value merely because it fits a field."
    } else {
        ""
    };
    let mixed = if profile["mixed"]["noul"].as_f64().is_some_and(|p| p >= 0.2) {
        "Potential mixed document or multiple people: keep each person, period and document boundary separate; leave uncertain ownership for review."
    } else {
        ""
    };
    let checklist = if guide.checklist.is_empty() {
        "Set slot to \"none\" for every fact.".to_owned()
    } else {
        let items: Vec<String> = guide
            .checklist
            .iter()
            .map(|c| format!("- {}: {} — {}", c.slot, c.label, c.description))
            .collect();
        format!(
            "Checklist. Look for each of these values and set slot to its key on the one fact that states it for this document's own period and person; set slot to \"none\" for every other fact, including cumulative totals and other people's values. Never omit a fact because it has no slot.\n{}",
            items.join("\n")
        )
    };
    format!("{layout}\n{mixed}\n{SAFETY}\n{EXTRACTION}\n{}\n{checklist}", guide.text)
}
```

`codex.rs`:

```rust
pub const PIPELINE: &str = "document-v6-guided-v1";

/// The extraction schema with `slot` limited to the guide's checklist plus `none`.
pub fn guided_schema(guide: &crate::guides::ReadingGuide, segments: &[String]) -> Value {
    let mut schema = output_schema();
    let item = &mut schema["properties"]["facts"]["items"];
    let mut slots = guide.slot_keys();
    slots.push("none".into());
    item["properties"]["slot"] = json!({"type":"string","enum":slots});
    item["required"].as_array_mut().expect("schema").push(json!("slot"));
    item["properties"]["segment_id"] = json!({"type":"string","enum":segments});
    schema
}
```

Remove `LEGACY_PIPELINE`. Thread `guide: &ReadingGuide` through `extract_document`, `extract`, `extract_with_binary`, `pipeline::run`, `run_with_decisions` and store it in `Runner` (`guide: &'a ReadingGuide`). In `Runner::model` replace the schema construction with `let schema = guided_schema(self.guide, &input.segments.iter().map(|s| s.segment_id.clone()).collect::<Vec<_>>());`. In `Runner::section`:
- `let instructions = document::instructions(&profile, self.guide);`
- add `request["guide"] = json!(self.guide.key); request["checklist"] = json!(self.guide.checklist);` before calling `self.model(input, "extract", …)` so the cache fingerprint changes with the guide.
- Checkpoint saving and restore accept facts whose only rejection is `subject_unknown` (ownership is decided later by TypeSafe):

```rust
        let owner_only = |r: &[me_core::RejectedFact]| r.iter().all(|q| q.code == "subject_unknown");
```

  use `owner_only(&checked.rejected)` instead of `checked.rejected.is_empty()` in the restore branch (returning `rejected: checked.rejected`) and before `self.checkpoints.save(…)`. Apply the same in `run_with_decisions` where a parent checkpoint is reused.
- Drop the `document_kind` question from `typesafe_questions::profile_questions` (Classify supplies the type).

Add the audit entry point in `codex_pipeline.rs` and export it from `codex.rs`:

```rust
/// One focused pass over lines the local sweep found uncovered. Its facts are
/// grounded like any other section.
pub(super) fn audit(
    home: &Path,
    input: &ExtractionInput,
    guide: &crate::guides::ReadingGuide,
    previous: &ExtractionOutput,
    uncovered: &[me_core::Uncovered],
    cancel: Arc<AtomicBool>,
    progress: &mut impl FnMut(Progress),
    binary: &Path,
    checkpoints: &mut impl Checkpoints,
) -> Result<ExtractionReport> {
    let ids: std::collections::BTreeSet<&str> = uncovered.iter().map(|u| u.segment_id.as_str()).collect();
    let mut part = input.with_segments(input.segments.iter().filter(|s| ids.contains(s.segment_id.as_str())).cloned().collect());
    let mut bytes = 0;
    part.segments.retain(|s| { bytes += s.text.len(); bytes <= 12_000 });
    let scratch = tempfile::Builder::new().prefix("me-inbox-").tempdir().map_err(failed)?;
    #[cfg(test)]
    let mut decisions = typesafe::FakeDecisions;
    #[cfg(not(test))]
    let mut decisions = typesafe::LazyDecisions::default();
    let mut runner = Runner { home, binary, scratch: scratch.path(), cancel, progress, checkpoints, decisions: &mut decisions, session: None, calls: 0, guide };
    let lines: Vec<Value> = uncovered.iter().map(|u| json!({"segment_id":u.segment_id,"label":u.label,"text":u.text,"line":u.line})).collect();
    let mut request = serde_json::to_value(&part).map_err(failed)?;
    request["stage"] = json!("sweep_audit");
    request["previous_facts"] = json!(previous.facts.iter().filter(|f| ids.contains(f.segment_id.as_str())).collect::<Vec<_>>());
    request["uncovered"] = json!(lines);
    request["guide"] = json!(guide.key);
    let instructions = format!(
        "{} Local checks found these printed values in `uncovered` that previous_facts do not quote. Re-read the source and return a fact for each one that is a real documented value, with its exact printed label. Return facts: [] for any that are not. Never infer unprinted values.",
        document::instructions(&json!({}), guide)
    );
    let output = runner.model(&part, "sweep_audit", request, &instructions)?;
    let checked = ground_extraction(&part, output);
    Ok(ExtractionReport { output: checked.output, rejected: checked.rejected, repaired: checked.repaired })
}
```

`codex.rs`:

```rust
pub fn audit_document(
    home: &Path,
    input: &ExtractionInput,
    guide: &crate::guides::ReadingGuide,
    previous: &ExtractionOutput,
    uncovered: &[me_core::Uncovered],
    cancel: Arc<AtomicBool>,
    mut progress: impl FnMut(Progress),
    checkpoints: &mut impl Checkpoints,
) -> Result<ExtractionReport> {
    pipeline::audit(home, input, guide, previous, uncovered, cancel, &mut progress, &executable(), checkpoints)
}
```

Update every existing call site in `me-agent` tests (`codex_batch_tests.rs`, `codex_setup_tests.rs`) to pass `&crate::guides::ReadingGuide::general()`.

- [ ] **Step 6: Run the tests**

Run: `./scripts/cargo test -p me-core && ./scripts/cargo test -p me-agent`
Expected: PASS, including the three guide tests and `the_extraction_schema_offers_only_checklist_slots`. The desktop crate will not compile until Task 8; do not run it yet.

- [ ] **Step 7: Commit**

```bash
./scripts/cargo fmt --all
git add crates/me-core crates/me-agent apps/desktop/examples
git commit -m "Guide the deep reader by document family, type and checklist

The reader now gets family and official-form guidance plus the
registry checklist and tags facts with a slot. Sections whose only
open question is ownership are checkpointed. A focused audit reads
lines the local sweep found uncovered.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: TypeSafe verification of every reader assumption

**Files:**
- Create: `crates/me-agent/src/fact_verification.rs`, `crates/me-agent/src/fact_verification_tests.rs`
- Modify: `crates/me-agent/src/typesafe_questions.rs` (question builders), `crates/me-agent/src/lib.rs` (`pub mod fact_verification;`)

**Interfaces:**
- Consumes: `Decisions`, `IdentityAnchors`, `Slot`, `ValueKind`, `PAYMENT_PERIODS`, `SLOT_FLOOR`, `VERIFY_FIRE`.
- Produces:

```rust
pub struct FactInput { pub label: String, pub value: String, pub quote: String, pub line: String, pub money: bool, pub slot: Option<&'static me_core::Slot>, pub category_unmapped: bool }
#[derive(Clone, Debug, PartialEq)] pub enum Owner { Anchor(String), Party(String), Organization, Unclear }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum PeriodKind { Document, Cumulative, Other, NotPeriodic }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Band { Accept, Check, Reject }
#[derive(Clone, Debug, PartialEq)]
pub struct Verdict { pub owner: Owner, pub period: Option<PeriodKind>, pub failure: f64, pub confidence: f64, pub band: Band,
    pub mapped: bool, pub refund: Option<bool>, pub payment_period: Option<me_core::Period>, pub category: Option<String> }
pub struct Verification { pub verdicts: Vec<Verdict>, pub correction: bool, pub usage: crate::graph_pipeline::StageUsage }
pub fn band(confidence: f64, failure: f64, threshold: f64) -> Band
pub fn verify_facts(title: &str, doc_label: &str, facts: &[FactInput], anchors: &me_core::IdentityAnchors, named: &[String], subject: Option<&str>, threshold: f64, decisions: &mut impl Decisions, cancel: &AtomicBool) -> crate::typesafe::Result<Verification>
// typesafe_questions.rs
pub const FACT_BATCH: usize = 20;
pub fn owner_options(anchors: &IdentityAnchors, named: &[String]) -> SubjectOptions // keys self, household_i, named_i, organization, unclear
pub fn fact_questions(batch: &[(usize, &FactInput)], owners: &SubjectOptions, ask_correction: bool) -> Value
pub fn competing_questions(slot: &str, facts: &[usize]) -> Value
```

- [ ] **Step 1: Write the failing tests** (`fact_verification_tests.rs`; copy the `Scripted` fake from `graph_pipeline_tests.rs` lines 11–72 into this file)

```rust
use super::*;
use crate::typesafe::{DecisionResponse, Decisions, Result};
use me_core::{HouseholdMember, IdentityAnchors};
use serde_json::{Map, Value, json};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};

// <Scripted fake copied verbatim from graph_pipeline_tests.rs>

fn anchors() -> IdentityAnchors {
    IdentityAnchors { self_entity: "me".into(), self_name: "Max Beispiel".into(), aliases: vec![], birth_date: None, household: Vec::<HouseholdMember>::new() }
}
fn payslip_fact(slot: &str, value: &str) -> FactInput {
    FactInput {
        label: "Lohnsteuer".into(), value: value.into(), quote: format!("Lohnsteuer   {value}"),
        line: format!("Lohnsteuer   {value}"), money: true,
        slot: me_core::doc_type("payslip").unwrap().slot(slot), category_unmapped: false,
    }
}
fn run(d: &mut Scripted, facts: &[FactInput]) -> Verification {
    verify_facts("payslip.pdf", "Payslip", facts, &anchors(), &[], Some("me"), 0.8, d, &AtomicBool::new(false)).unwrap()
}

#[test]
fn confident_assumptions_are_accepted_silently() {
    let mut d = Scripted::default()
        .choice("f0::owner", "self", 0.95).choice("f0::period", "document", 0.93).noul("f0::mapping", 0.92);
    let v = run(&mut d, &[payslip_fact("wage_tax", "1.032,58")]);
    assert_eq!(v.verdicts[0].band, Band::Accept);
    assert!(v.verdicts[0].mapped);
    assert_eq!(v.verdicts[0].owner, Owner::Anchor("me".into()));
}

#[test]
fn a_doubtful_assumption_becomes_a_check_and_a_wrong_one_is_rejected() {
    let mut d = Scripted::default()
        .choice("f0::owner", "self", 0.72).choice("f0::period", "document", 0.9).noul("f0::mapping", 0.9)
        .choice("f1::owner", "self", 0.95).choice("f1::period", "document", 0.9).noul("f1::mapping", 0.9).noul("f1::off_target", 0.8);
    let v = run(&mut d, &[payslip_fact("wage_tax", "1.032,58"), payslip_fact("church_tax", "0,00")]);
    assert_eq!(v.verdicts[0].band, Band::Check);
    assert!(v.verdicts[0].mapped);
    assert_eq!(v.verdicts[1].band, Band::Reject);
    assert!(!v.verdicts[1].mapped);
}

#[test]
fn a_year_to_date_total_or_another_persons_value_never_enters_the_monthly_slot() {
    let mut d = Scripted::default()
        .choice("f0::owner", "self", 0.95).choice("f0::period", "cumulative", 0.95).noul("f0::mapping", 0.9)
        .choice("f1::owner", "organization", 0.95).choice("f1::period", "document", 0.95).noul("f1::mapping", 0.9);
    let v = run(&mut d, &[payslip_fact("wage_tax", "12.390,96"), payslip_fact("gross", "5.340,00")]);
    assert!(!v.verdicts[0].mapped && !v.verdicts[1].mapped);
    assert_eq!(v.verdicts[0].band, Band::Accept, "the fact itself stays a verified document fact");
}

#[test]
fn facts_are_verified_in_batches_of_twenty() {
    let facts: Vec<FactInput> = (0..45).map(|i| FactInput { slot: None, ..payslip_fact("wage_tax", &format!("{i},00")) }).collect();
    let mut d = Scripted::default();
    let v = run(&mut d, &facts);
    assert_eq!(d.requests.len(), 3);
    assert_eq!(v.verdicts.len(), 45);
    assert!(d.requests[0].1.as_object().unwrap().contains_key("document::correction"));
    assert!(!d.requests[1].1.as_object().unwrap().contains_key("document::correction"));
}

#[test]
fn competing_mappings_are_decided_by_a_choice_over_the_readers_facts() {
    let mut d = Scripted::default()
        .choice("f0::owner", "self", 0.95).choice("f0::period", "document", 0.95).noul("f0::mapping", 0.9)
        .choice("f1::owner", "self", 0.95).choice("f1::period", "document", 0.95).noul("f1::mapping", 0.9)
        .choice("slot_net", "f1", 0.9);
    let v = run(&mut d, &[payslip_fact("net", "3.210,05"), payslip_fact("net", "3.210,05 ")]);
    assert_eq!(d.requests.len(), 2);
    assert!(!v.verdicts[0].mapped && v.verdicts[1].mapped);
}

#[test]
fn a_balance_direction_and_a_correction_are_judged_not_computed() {
    let slot = me_core::doc_type("tax_assessment").unwrap().slot("tax_balance");
    let fact = FactInput { slot, label: "Erstattung".into(), ..payslip_fact("gross", "812,00") };
    let mut d = Scripted::default()
        .choice("f0::owner", "self", 0.95).choice("f0::period", "not_periodic", 0.9).noul("f0::mapping", 0.9)
        .choice("f0::direction", "refund", 0.93).noul("document::correction", 0.9);
    let v = verify_facts("bescheid.pdf", "Tax assessment", &[fact], &anchors(), &[], Some("me"), 0.8, &mut d, &AtomicBool::new(false)).unwrap();
    assert_eq!(v.verdicts[0].refund, Some(true));
    assert!(v.verdicts[0].mapped);
    assert!(v.correction);
}

#[test]
fn bands_follow_the_documented_thresholds() {
    assert_eq!(band(0.95, 0.1, 0.8), Band::Accept);
    assert_eq!(band(0.75, 0.1, 0.8), Band::Check);
    assert_eq!(band(0.55, 0.1, 0.8), Band::Reject);
    assert_eq!(band(0.95, 0.71, 0.8), Band::Reject);
}
```

Note for `a_balance_direction…`: a `Balance` slot is non-periodic, so the period rule for mapping requires `NotPeriodic` or `Document`.

- [ ] **Step 2: Run to verify failure**

Run: `./scripts/cargo test -p me-agent fact_verification`
Expected: FAIL to compile.

- [ ] **Step 3: Add the question builders** (`typesafe_questions.rs`, after `verify_questions`)

```rust
// Verification of reader facts. Every reader assumption is asked; code combines
// the answers into one confidence (the weakest head) and a band.
/// Facts per verification request.
pub const FACT_BATCH: usize = 20;

/// Owner options: the anchors, people named in the document, an organization, unclear.
pub fn owner_options(anchors: &IdentityAnchors, named: &[String]) -> SubjectOptions {
    let mut options = subject_options(anchors, named);
    options.criteria.remove("none");
    options.criteria.insert("organization".into(), json!("An organization's own detail (employer, insurer, bank, authority), not a private person's"));
    options.criteria.insert("unclear".into(), json!("The document does not say whose detail this is"));
    options
}

pub fn fact_questions(
    batch: &[(usize, &crate::fact_verification::FactInput)],
    owners: &SubjectOptions,
    ask_correction: bool,
) -> Value {
    let mut q = Map::new();
    let heads = [
        ("hallucinated", "Is `facts.{f}.value` unsupported by `facts.{f}.quote`, or not printed in `document` as the value of `facts.{f}.label`?"),
        ("off_target", "Does `facts.{f}.label` fail to describe what `facts.{f}.value` is in `document`: for example a year-to-date total taken as a monthly amount, an employer share taken as the employee's, or a value from a different row or column?"),
    ];
    for (i, fact) in batch {
        let f = format!("f{i}");
        for (head, text) in heads {
            q.insert(format!("{f}::{head}"), json!({"type":"noul","instructions":text.replace("{f}", &f),"criteria":{"true":"The fact is wrong in this way","false":"The fact is fine in this respect"}}));
        }
        q.insert(format!("{f}::owner"), choice(json!(format!("Whose detail is `facts.{f}` according to `document`?")), owners.criteria.clone()));
        if fact.money {
            q.insert(format!("{f}::period"), choice(json!(format!("Which period does the amount `facts.{f}` cover?")), json!({
                "document":"The period this document is for, such as this payslip's month or this certificate's year",
                "cumulative":"A cumulative or year-to-date total",
                "other":"Another stated period: a correction month, a previous year or a future instalment",
                "not_periodic":"Not a periodic amount: a one-off amount, a balance or a limit"}).as_object().cloned().unwrap_or_default()));
        }
        if let Some(slot) = fact.slot {
            q.insert(format!("{f}::mapping"), noul(json!(format!("Is `facts.{f}` the `slots.{}` of `document`?", slot.key))));
            match slot.value {
                ValueKind::Balance => {
                    q.insert(format!("{f}::direction"), choice(json!(format!("Does `facts.{f}` state money paid back to the taxpayer or money the taxpayer must pay?")), json!({
                        "refund":"Paid back to the taxpayer (Erstattung, Guthaben)",
                        "payment":"The taxpayer must pay it (Nachzahlung, zu zahlen)",
                        "unclear":"The document does not say"}).as_object().cloned().unwrap_or_default()));
                }
                ValueKind::Money(None) => {
                    let periods = me_core::PAYMENT_PERIODS.iter().map(|(k, d)| ((*k).to_owned(), json!(d))).collect();
                    q.insert(format!("{f}::payment_period"), choice(json!(format!("How often is `facts.{f}` paid?")), periods));
                }
                ValueKind::Category(options) if fact.category_unmapped => {
                    let criteria = options.iter().map(|(k, d)| ((*k).to_owned(), json!(d))).collect();
                    q.insert(format!("{f}::category"), choice(json!(format!("Which option describes `facts.{f}`?")), criteria));
                }
                _ => {}
            }
        }
    }
    if ask_correction {
        q.insert("document::correction".into(), noul(json!("Does `document` state that it corrects, replaces or cancels an earlier document of the same kind (Korrektur, Berichtigung, Stornierung, Nachberechnung, geändert)?")));
    }
    json!(q)
}

/// Several facts claim one slot: which one is it?
pub fn competing_questions(slot: &str, facts: &[usize]) -> Value {
    let mut criteria: Map<String, Value> = facts.iter().map(|i| (format!("f{i}"), json!(format!("`facts.f{i}`")))).collect();
    criteria.insert("none".into(), json!("None of these is the value"));
    json!({format!("slot_{slot}"): choice(json!(format!("Which fact is the `slots.{slot}` of `document`?")), criteria)})
}
```

- [ ] **Step 4: Implement `fact_verification.rs`**

```rust
//! TypeSafe validation of every assumption the reader made about a fact: that
//! the value means what its label says, whose it is, which period it covers and
//! which profile slot it fills. Code combines the answers and decides the band.
use crate::graph_pipeline::StageUsage;
use crate::typesafe::{Decisions, Result};
use crate::typesafe_questions as q;
use me_core::{IdentityAnchors, Period, Slot, Target, ValueKind};
use serde_json::{Value, json};
use std::sync::atomic::AtomicBool;

pub struct FactInput {
    pub label: String,
    pub value: String,
    pub quote: String,
    pub line: String,
    /// The value parses as an amount; the period question applies.
    pub money: bool,
    /// Slot the reader tagged and the value type-checked for.
    pub slot: Option<&'static Slot>,
    /// A category slot whose printed value code could not map.
    pub category_unmapped: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Owner { Anchor(String), Party(String), Organization, Unclear }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeriodKind { Document, Cumulative, Other, NotPeriodic }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Band { Accept, Check, Reject }

#[derive(Clone, Debug, PartialEq)]
pub struct Verdict {
    pub owner: Owner,
    pub period: Option<PeriodKind>,
    /// Highest failure-mode probability (true means wrong).
    pub failure: f64,
    /// Weakest head.
    pub confidence: f64,
    pub band: Band,
    /// The fact fills its slot in the profile.
    pub mapped: bool,
    pub refund: Option<bool>,
    pub payment_period: Option<Period>,
    pub category: Option<String>,
}
pub struct Verification {
    pub verdicts: Vec<Verdict>,
    pub correction: bool,
    pub usage: StageUsage,
}

pub fn band(confidence: f64, failure: f64, threshold: f64) -> Band {
    if failure > q::VERIFY_FIRE || confidence < q::SLOT_FLOOR {
        Band::Reject
    } else if confidence < threshold {
        Band::Check
    } else {
        Band::Accept
    }
}

fn invalid() -> me_core::ImportFailure {
    me_core::ImportFailure::new(me_core::ImportProvider::TypeSafe, me_core::ImportErrorKind::InvalidOutput,
        "TypeSafe returned an invalid decision. Saved steps are kept; retry explicitly.")
}
fn chosen(a: &Value, key: &str) -> Result<Option<(String, f64)>> {
    let Some(v) = a.get(key) else { return Ok(None) };
    Ok(Some((v["choice"].as_str().ok_or_else(invalid)?.to_owned(), v["confidence"].as_f64().ok_or_else(invalid)?)))
}
fn noul(a: &Value, key: &str) -> Result<Option<f64>> {
    match a.get(key) { None => Ok(None), Some(v) => Ok(Some(v["noul"].as_f64().ok_or_else(invalid)?)) }
}

fn state(title: &str, doc_label: &str, batch: &[(usize, &FactInput)]) -> Value {
    let mut facts = serde_json::Map::new();
    let mut slots = serde_json::Map::new();
    for (i, f) in batch {
        facts.insert(format!("f{i}"), json!({"label":f.label,"value":f.value,"quote":f.quote,"line":f.line}));
        if let Some(s) = f.slot {
            slots.insert(s.key.to_owned(), json!({"label":s.label,"description":s.description}));
        }
    }
    json!({"document":{"file_name":title,"type":doc_label},"facts":facts,"slots":slots})
}

#[allow(clippy::too_many_arguments)]
pub fn verify_facts(
    title: &str,
    doc_label: &str,
    facts: &[FactInput],
    anchors: &IdentityAnchors,
    named: &[String],
    subject: Option<&str>,
    threshold: f64,
    decisions: &mut impl Decisions,
    cancel: &AtomicBool,
) -> Result<Verification> {
    let owners = q::owner_options(anchors, named);
    let mut usage = StageUsage::default();
    let mut verdicts = Vec::with_capacity(facts.len());
    let mut correction = false;
    let indexed: Vec<(usize, &FactInput)> = facts.iter().enumerate().collect();
    for (n, batch) in indexed.chunks(q::FACT_BATCH).enumerate() {
        let response = decisions.evaluate(state(title, doc_label, batch), q::fact_questions(batch, &owners, n == 0), cancel)?;
        usage.add(&response);
        let a = &response.answers;
        if n == 0 {
            correction = noul(a, "document::correction")?.is_some_and(|p| p >= q::PRESENT_FOUND);
        }
        for (i, fact) in batch {
            let f = format!("f{i}");
            let failure = ["hallucinated", "off_target"].iter()
                .map(|h| noul(a, &format!("{f}::{h}")))
                .collect::<Result<Vec<_>>>()?.into_iter().flatten().fold(0., f64::max);
            let (owner_key, owner_conf) = chosen(a, &format!("{f}::owner"))?.ok_or_else(invalid)?;
            let owner = if let Some((_, e)) = owners.anchors.iter().find(|(k, _)| *k == owner_key) {
                Owner::Anchor(e.clone())
            } else if let Some((_, name)) = owners.named.iter().find(|(k, _)| *k == owner_key) {
                Owner::Party(name.clone())
            } else if owner_key == "organization" { Owner::Organization } else { Owner::Unclear };
            let period = chosen(a, &format!("{f}::period"))?;
            let period_kind = period.as_ref().map(|(k, _)| match k.as_str() {
                "document" => PeriodKind::Document, "cumulative" => PeriodKind::Cumulative,
                "other" => PeriodKind::Other, _ => PeriodKind::NotPeriodic });
            let mapping = noul(a, &format!("{f}::mapping"))?;
            let mut confidence = (1. - failure).min(owner_conf);
            if let Some((_, c)) = &period { confidence = confidence.min(*c); }
            if let Some(m) = mapping { confidence = confidence.min(m); }
            let fact_band = band(confidence, failure, threshold);
            let refund = chosen(a, &format!("{f}::direction"))?.and_then(|(k, c)| (c >= q::SLOT_FLOOR).then_some(k)).and_then(|k| match k.as_str() { "refund" => Some(true), "payment" => Some(false), _ => None });
            let payment_period = chosen(a, &format!("{f}::payment_period"))?.and_then(|(k, c)| (c >= q::SLOT_FLOOR).then_some(k)).and_then(|k| match k.as_str() {
                "month" => Some(Period::Month), "quarter" => Some(Period::Quarter), "half_year" => Some(Period::HalfYear), "year" => Some(Period::Year), _ => None });
            let category = chosen(a, &format!("{f}::category"))?.filter(|(k, c)| *c >= q::SLOT_FLOOR && k != "other").map(|(k, _)| k);
            let mapped = fact.slot.is_some_and(|slot| {
                let owner_ok = match slot.target {
                    Target::Subject => matches!(&owner, Owner::Anchor(e) if Some(e.as_str()) == subject),
                    Target::Role(_) => true,
                };
                let period_ok = match slot.value {
                    ValueKind::Money(Some(_)) => period_kind == Some(PeriodKind::Document),
                    ValueKind::Balance => matches!(period_kind, Some(PeriodKind::Document | PeriodKind::NotPeriodic)) && refund.is_some(),
                    ValueKind::Money(None) => period_kind != Some(PeriodKind::Cumulative),
                    ValueKind::Category(_) => !fact.category_unmapped || category.is_some(),
                    _ => true,
                };
                owner_ok && period_ok && mapping.is_some_and(|m| m >= q::SLOT_FLOOR) && fact_band != Band::Reject
            });
            verdicts.push(Verdict { owner, period: period_kind, failure, confidence, band: fact_band, mapped, refund, payment_period, category });
        }
    }
    // Competing mappings: one Choice per contested slot over the reader's facts.
    let mut contested: std::collections::BTreeMap<&str, Vec<usize>> = Default::default();
    for (i, (fact, v)) in facts.iter().zip(&verdicts).enumerate() {
        if let (Some(slot), true) = (fact.slot, v.mapped) {
            contested.entry(slot.key).or_default().push(i);
        }
    }
    for (slot, ids) in contested.into_iter().filter(|(_, ids)| ids.len() > 1) {
        let batch: Vec<(usize, &FactInput)> = ids.iter().map(|i| (*i, &facts[*i])).collect();
        let response = decisions.evaluate(state(title, doc_label, &batch), q::competing_questions(slot, &ids), cancel)?;
        usage.add(&response);
        let (winner, c) = chosen(&response.answers, &format!("slot_{slot}"))?.ok_or_else(invalid)?;
        for i in ids {
            if winner != format!("f{i}") || c < q::SLOT_FLOOR {
                verdicts[i].mapped = false;
            }
        }
    }
    Ok(Verification { verdicts, correction, usage })
}

#[cfg(test)]
#[path = "fact_verification_tests.rs"]
mod tests;
```

Make `StageUsage::add` `pub(crate)` in `graph_pipeline.rs` (it is a private `fn add` today).

- [ ] **Step 5: Run the tests**

Run: `./scripts/cargo test -p me-agent fact_verification`
Expected: PASS (7 tests).

- [ ] **Step 6: Commit**

```bash
./scripts/cargo fmt --all
git add crates/me-agent
git commit -m "Verify every reader assumption with batched TypeSafe questions

Meaning, owner, period and profile mapping are asked per fact; the
weakest answer decides accept, check or reject. Contested slots, refund
direction, payment period and corrections are judged, never computed.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: One read pipeline for automatic and manual runs

**Files:**
- Create: `apps/desktop/src/read_import.rs`
- Delete: `apps/desktop/src/graph_import.rs`
- Modify: `apps/desktop/src/ai_ui.rs` (both branches call `run_read`; `VaultCheckpoints::load` without legacy fallback), `apps/desktop/src/main.rs` or the module list that declares `graph_import` (rename to `read_import`)
- Modify: `crates/me-agent/src/graph_pipeline.rs` (delete `extract`, `verify_gap_fill`, `extract_state`, `Extraction`, `mrz_value` moves to `read_import.rs`), `graph_pipeline_tests.rs` (delete tests of removed functions), `typesafe_questions.rs` (delete `extract_questions`, `verify_questions`, `MAX_SLOT_OPTIONS`, `MAX_EXTRACT_STATE`, `PRESENT_ABSENT` if unused)

**Interfaces:**
- Consumes: everything from Tasks 1–7.
- Produces:

```rust
pub(super) struct ReadRun { pub outcome: me_core::ReadOutcome, pub family: String }
pub(super) fn run_read(session: &Arc<Mutex<Option<Vault>>>, item: u64, run: &str, cancel: &Arc<AtomicBool>, tx: &Sender<Progress>, home: &Path) -> Result<ReadRun, String>
```

- [ ] **Step 1: Write the orchestration**

`read_import.rs` keeps `vault_call`, `step`, `Metered` and `time_year` from `graph_import.rs` (move them verbatim) and adds:

```rust
//! Worker side of deep-first reading, for automatic and manual runs alike:
//! Classify (TypeSafe) → guided reader (OpenAI, grounded) → omission sweep and
//! one focused audit → local typing → TypeSafe verification → Resolve and the
//! read store. The vault mutex is held only for short calls.
use super::*;
use me_agent::codex::Progress;
use me_agent::fact_verification::{self as verify, Band, FactInput, Owner, PeriodKind};
use me_agent::graph_pipeline::{self as pipeline, Classification};
use me_core::{
    Candidate, CandidateValue, ConfidenceSource, DocumentGraph, DocumentRead, FactState,
    ImportErrorKind as Kind, ImportFailure, ImportProvider, ImportStage, ReadFact, SlotContent,
    SlotValue, SourceSegment, TypingContext, ValueKind, find_candidates, locate, type_value,
    uncovered,
};
use std::sync::{atomic::{AtomicBool, Ordering}, mpsc::Sender};

pub(super) struct ReadRun {
    pub outcome: me_core::ReadOutcome,
    pub family: String,
}

/// Forwards reader progress into the vault and the UI channel; a failed write
/// stops the attempt instead of continuing unrecorded.
fn forward(
    session: &Arc<Mutex<Option<Vault>>>,
    item: u64,
    run: &str,
    tx: &Sender<Progress>,
    cancel: &Arc<AtomicBool>,
    failed: &mut Option<String>,
    event: Progress,
) {
    let saved = (|| -> Result<(), String> {
        match &event {
            Progress::Step { stage, current, total } => {
                if !vault_call(session, |v| v.update_import_progress(item, run, *stage, *current, *total))? {
                    return Err("This import attempt is no longer active.".into());
                }
            }
            Progress::Failure(f) => vault_call(session, |v| v.record_import_failure(item, run, f))?,
            Progress::Usage { call, input_tokens, output_tokens } => {
                if !vault_call(session, |v| v.record_import_usage(item, run, call, *input_tokens, *output_tokens))? {
                    return Err("This import attempt is no longer active.".into());
                }
            }
            _ => {}
        }
        Ok(())
    })();
    if let Err(e) = saved {
        *failed = Some(e);
        cancel.store(true, Ordering::SeqCst);
    }
    let _ = tx.send(event);
}
```

The body of `run_read` (write it as one function with these numbered blocks; every vault access goes through `vault_call`):

```rust
pub(super) fn run_read(
    session: &Arc<Mutex<Option<Vault>>>,
    item: u64,
    run: &str,
    cancel: &Arc<AtomicBool>,
    tx: &Sender<Progress>,
    home: &std::path::Path,
) -> Result<ReadRun, String> {
    // 1. Inputs.
    let (source, title, texts, anchors, threshold, fingerprint, batch) = vault_call(session, |v| {
        let (source, title, texts) = v.document_texts(item)?;
        let fingerprint = v.source_fingerprint(&source)?;
        Ok((source, title, texts, v.identity_anchors()?, v.check_threshold()?, fingerprint, v.item_batch(item)?))
    })?;
    let segments: Vec<SourceSegment<'_>> = texts.iter().map(|t| SourceSegment { id: &t.segment_id, text: &t.text }).collect();
    let names = anchors.names();
    let name_refs: Vec<&str> = names.iter().map(|(_, n)| n.as_str()).collect();
    let year = time_year();
    let candidates = find_candidates(&segments, &name_refs, year);
    let mut decisions = Metered { session, item, run, inner: me_agent::typesafe::TypeSafe::configured().map_err(|f| f.message)?, tx };
    let fail = |f: ImportFailure| {
        let _ = vault_call(session, |v| v.record_import_failure(item, run, &f));
        let _ = tx.send(Progress::Failure(f.clone()));
        f.message
    };

    // 2. Classify (cached by content), exactly as the tiered path did.
    step(tx, session, item, run, ImportStage::Interpreting, false);
    let classification: Classification = /* move the cached-classify block from graph_import.rs verbatim */;
    vault_call(session, |v| v.save_document_profile(&source, &classification.family, classification.family_confidence,
        classification.doc_type.as_deref(), classification.type_confidence, classification.subject.as_deref(),
        classification.subject_name.as_deref(), classification.usage.models.first().map(String::as_str)))?;
    step(tx, session, item, run, ImportStage::Interpreting, true);
    let kind = classification.doc_type.as_deref().and_then(me_core::doc_type);
    let guide = me_agent::guides::reading_guide(&classification.family, classification.doc_type.as_deref());

    // 3. Guided reading over the whole document (checkpointed, allowance-reserved).
    let input = vault_call(session, |v| v.prepare_extraction(item, me_agent::codex::INBOX_MODEL))?;
    let mut failed = None;
    let mut checkpoints = super::ai_ui::VaultCheckpoints { session: session.clone() };
    let report = me_agent::codex::extract_document(home, &input, &guide, cancel.clone(),
        |e| forward(session, item, run, tx, cancel, &mut failed, e), &mut checkpoints);
    if let Some(e) = failed.take() { let _ = vault_call(session, |v| v.fail_extraction(&input.run_id)); return Err(e); }
    let report = match report { Ok(r) => r, Err(e) => { let _ = vault_call(session, |v| v.fail_extraction(&input.run_id)); return Err(e); } };
    let mut rejected = std::collections::BTreeMap::<String, usize>::new();
    let mut facts = report.output.facts;
    for r in report.rejected {
        if r.code == "subject_unknown" { facts.push(r.fact); } else { *rejected.entry(r.code.to_owned()).or_default() += 1; }
    }

    // 4. Omission sweep and at most one focused audit.
    let covered = |facts: &[me_core::ExtractedFact]| -> Vec<(String, usize, usize)> {
        facts.iter().filter_map(|f| locate(&segments, &f.segment_id, &f.quote, &f.value)).map(|l| (l.segment_id, l.quote_start, l.quote_end)).collect()
    };
    let mut open = uncovered(&candidates, &covered(&facts));
    let missing_required = kind.is_some_and(|k| k.slots.iter().any(|s| s.required && !facts.iter().any(|f| f.slot == s.key)));
    if !open.is_empty() || missing_required {
        let _ = tx.send(Progress::Message(format!("Checking {} value(s) the first pass did not quote", open.len())));
        let previous = me_core::ExtractionOutput { facts: facts.clone() };
        let audit = me_agent::codex::audit_document(home, &input, &guide, &previous, &open, cancel.clone(),
            |e| forward(session, item, run, tx, cancel, &mut failed, e), &mut checkpoints);
        if let Some(e) = failed.take() { let _ = vault_call(session, |v| v.fail_extraction(&input.run_id)); return Err(e); }
        match audit {
            Ok(a) => {
                for f in a.output.facts { me_core::merge_extracted_fact(&mut facts, f); }
                for r in a.rejected {
                    if r.code == "subject_unknown" { me_core::merge_extracted_fact(&mut facts, r.fact); }
                    else { *rejected.entry(r.code.to_owned()).or_default() += 1; }
                }
                open = uncovered(&candidates, &covered(&facts));
            }
            Err(e) => { let _ = vault_call(session, |v| v.fail_extraction(&input.run_id)); return Err(e); }
        }
    }

    // 5. Local typing.
    let typing = TypingContext::new(&segments, classification.doc_type.as_deref(), year);
    struct Typed { at: me_core::Located, candidate: Option<Candidate>, category_unmapped: bool }
    let mut typed = Vec::new();
    let mut inputs = Vec::new();
    for f in &facts {
        let Some(at) = locate(&segments, &f.segment_id, &f.quote, &f.value) else { continue };
        let label = me_core::document_field_label(&f.property).map(str::to_owned)
            .or_else(|| me_core::standard_field(&f.property).map(|s| s.label.to_owned()))
            .unwrap_or_else(|| f.property.clone());
        let slot = kind.and_then(|k| k.slot(&f.slot)).filter(|s| s.mrz.is_none());
        let candidate = slot.and_then(|s| type_value(&typing, &at, &label, &f.value, s.value));
        let category_unmapped = matches!(slot.map(|s| s.value), Some(ValueKind::Category(_))) && candidate.is_none();
        let slot = if candidate.is_some() || category_unmapped { slot } else { None };
        inputs.push(FactInput { label, value: f.value.clone(), quote: f.quote.clone(), line: at.line.clone(),
            money: me_core::amount_of(&typing, &f.value).is_some(), slot, category_unmapped });
        typed.push(Typed { at, candidate, category_unmapped });
    }

    // 6. TypeSafe verification. A failure keeps the facts unverified and resumable.
    step(tx, session, item, run, ImportStage::Verifying, false);
    let named: Vec<String> = candidates.iter().filter(|c| c.kind == me_core::CandidateKind::PersonName)
        .map(|c| c.text.trim().to_owned()).collect::<std::collections::BTreeSet<_>>().into_iter().take(24).collect();
    let subject = classification.subject.clone().or_else(|| Some(anchors.self_entity.clone()));
    let verification = verify::verify_facts(&title, kind.map_or("Document", |k| k.label), &inputs, &anchors, &named,
        subject.as_deref(), threshold, &mut decisions, cancel);
    let verification = match verification {
        Ok(v) => v,
        Err(f) => {
            let read = DocumentRead {
                run_id: input.run_id.clone(), graph: None, rejected: rejected.clone(),
                facts: inputs.iter().zip(&typed).map(|(i, t)| ReadFact { label: i.label.clone(), value: i.value.clone(),
                    segment_id: t.at.segment_id.clone(), start: t.at.start, end: t.at.end, context: String::new(),
                    owner: "unknown".into(), owner_entity: None, owner_name: None, period: None, slot: None,
                    state: FactState::Unverified, confidence: None }).collect(),
                uninterpreted: open.clone(),
            };
            let _ = vault_call(session, |v| v.apply_read(&source, &read));
            return Err(fail(f));
        }
    };

    // 7. Profile values from verified mappings, plus MRZ fields by position.
    let mut values: Vec<SlotValue> = Vec::new();
    if let Some(k) = kind {
        if let Some(mrz) = candidates.iter().find(|c| matches!(&c.value, CandidateValue::Mrz(m) if m.check_digits_valid)) {
            for slot in k.slots {
                if let Some(field) = slot.mrz && let Some(c) = mrz_value(mrz, field) {
                    values.push(SlotValue { slot: slot.key.into(), content: SlotContent::Candidate(Box::new(c)), period: None,
                        confidence: 1., source: ConfidenceSource::Checksum, value_checked: true, check: false });
                }
            }
        }
        for ((input, t), v) in inputs.iter().zip(&typed).zip(&verification.verdicts) {
            let (Some(slot), true) = (input.slot, v.mapped) else { continue };
            if values.iter().any(|x| x.slot == slot.key) { continue; }
            let content = match (&t.candidate, &v.category) {
                (_, Some(key)) if t.category_unmapped => SlotContent::Category { key: key.clone() },
                (Some(c), _) => {
                    let mut c = c.clone();
                    if v.refund == Some(true) && let CandidateValue::Money { amount, .. } = &mut c.value && !amount.starts_with('-') {
                        amount.insert(0, '-');
                    }
                    SlotContent::Candidate(Box::new(c))
                }
                _ => continue,
            };
            let checked = t.candidate.as_ref().is_some_and(|c| c.checksum);
            values.push(SlotValue { slot: slot.key.into(), content, period: v.payment_period, confidence: v.confidence,
                source: ConfidenceSource::Typesafe, value_checked: checked, check: v.band == Band::Check });
        }
        // Pay month fallback: the shared period of accepted monthly values.
        if let Some(slot) = k.slots.iter().find(|s| s.value == ValueKind::Period) && !values.iter().any(|x| x.slot == slot.key) {
            let periods: std::collections::BTreeSet<(String, String)> = facts.iter().zip(&inputs).zip(&verification.verdicts)
                .filter(|((_, i), v)| v.mapped && i.money && v.period == Some(PeriodKind::Document))
                .filter_map(|((f, _), _)| {
                    let at = locate(&segments, &f.segment_id, &f.quote, &f.context_quote).or_else(|| {
                        segments.iter().find_map(|s| s.text.find(&f.context_quote).map(|_| ()))?;
                        None
                    })?;
                    match type_value(&typing, &at, "period", &f.context_quote, ValueKind::Period)?.value {
                        CandidateValue::Period { start, end } => Some((start, end)),
                        _ => None,
                    }
                })
                .collect();
            if periods.len() == 1 {
                let (start, end) = periods.into_iter().next().unwrap_or_default();
                let text = facts.iter().find(|f| !f.context_quote.is_empty()).map(|f| f.context_quote.clone()).unwrap_or_default();
                values.push(SlotValue { slot: slot.key.into(), content: SlotContent::Candidate(Box::new(Candidate {
                    id: String::new(), kind: me_core::CandidateKind::Period, text: text.clone(),
                    value: CandidateValue::Period { start, end }, segment_id: String::new(), start: 0, end: 0,
                    line: text, label: Some("period".into()), checksum: false })), period: None, confidence: 0.9,
                    source: ConfidenceSource::Typesafe, value_checked: false, check: false });
            }
        }
    }
    let graph = kind.filter(|k| k.tier == me_core::Tier::Eager).map(|k| DocumentGraph {
        doc_type: k.key.into(), subject: subject.clone(), subject_confidence: classification.subject_confidence.max(0.5),
        values, models: verification.usage.models.clone(), correction: verification.correction });

    // 8. Persist.
    let read = DocumentRead {
        run_id: input.run_id.clone(),
        graph,
        facts: inputs.iter().zip(&typed).zip(&verification.verdicts).zip(&facts).map(|(((i, t), v), f)| {
            let (owner, owner_entity, owner_name) = match &v.owner {
                Owner::Anchor(e) if *e == anchors.self_entity => ("self", Some(e.clone()), None),
                Owner::Anchor(e) => ("household", Some(e.clone()), None),
                Owner::Party(n) => ("party", None, Some(n.clone())),
                Owner::Organization => ("organization", None, None),
                Owner::Unclear => ("unclear", None, None),
            };
            ReadFact { label: i.label.clone(), value: i.value.clone(), segment_id: t.at.segment_id.clone(),
                start: t.at.start, end: t.at.end, context: f.context_quote.clone(), owner: owner.into(), owner_entity, owner_name,
                period: v.period.map(|p| match p { PeriodKind::Document => "document", PeriodKind::Cumulative => "cumulative",
                    PeriodKind::Other => "other", PeriodKind::NotPeriodic => "none" }.to_owned()),
                slot: v.mapped.then(|| i.slot.map(|s| s.key.to_owned())).flatten(),
                state: if v.band == Band::Accept { FactState::Verified } else { FactState::Uncertain },
                confidence: Some(v.confidence) }
        }).collect(),
        uninterpreted: open,
        rejected,
    };
    let outcome = vault_call(session, |v| v.apply_read(&source, &read))?;
    vault_call(session, |v| v.complete_read_run(&input.run_id))?;
    step(tx, session, item, run, ImportStage::Verifying, true);
    if let Some(batch) = &batch {
        let _ = vault_call(session, |v| v.record_batch_usage(batch, verification.usage.requests, verification.usage.input_tokens, 0));
    }
    let _ = fingerprint;
    Ok(ReadRun { outcome, family: classification.family })
}
```

Simplify the pay-month fallback while implementing: locate the `context_quote` directly with `me_core::locate(&segments, &f.segment_id, &f.context_quote, &f.context_quote)` and, if that fails, search every segment for it; the block above shows the intent (one shared period across accepted monthly values → `pay_month`). Keep the period candidate's real span when located.

Move `mrz_value` (from `graph_pipeline.rs`) into `read_import.rs` as a private fn. Make `VaultCheckpoints` `pub(super)` in `ai_ui.rs` and remove the `LEGACY_PIPELINE` fallback from its `load`.

- [ ] **Step 2: Call it from `ai_ui.rs`**

In `start_document`, replace both the `if automatic { … graph_import … }` branch and the manual `prepare_extraction … finish_extraction_with_questions` block with one call:

```rust
                    stage = "read.pipeline";
                    record(stage, &[F::Count("item", item), F::Flag("automatic", automatic)]);
                    let read = super::read_import::run_read(&session, item, &run, &cancel, &tx, &home);
                    let mut guard = session.lock().map_err(|_| "Vault unavailable.")?;
                    let v = guard.as_mut().ok_or("Vault locked.")?;
                    match read {
                        Ok(read) => Ok(DocumentResult {
                            collection: v.collection("", false).map_err(core_error)?,
                            proposals: v.proposals(item).map_err(core_error)?,
                            fact_count: Some(read.outcome.values),
                            warning: (read.outcome.checks > 0).then(|| format!("{} worth a quick look in Review.",
                                if read.outcome.checks == 1 { "1 value is".to_owned() } else { format!("{} values are", read.outcome.checks) })),
                            graph: Some(read),
                        }),
                        Err(e) => {
                            let failure = me_core::ImportFailure::new(me_core::ImportProvider::Local,
                                if cancel.load(Ordering::SeqCst) { me_core::ImportErrorKind::Cancelled } else { me_core::ImportErrorKind::Storage }, e.clone());
                            let _ = v.record_import_failure(item, &run, &failure);
                            Err(e)
                        }
                    }
```

`DocumentResult.graph` becomes `Option<super::read_import::ReadRun>`. The completion message becomes:

```rust
                        this.ai_message = Some(match (&graph, count) {
                            (Some(r), _) => format!("{} values read · {} in your profile", r.outcome.values, r.outcome.in_profile),
                            (None, None) => "File is searchable. Ready for AI analysis.".into(),
                            (None, Some(_)) => "Analysis complete.".into(),
                        });
```

Do not overwrite a recorded provider failure with the local one: keep the existing pattern (`record_import_failure` is idempotent per run; the provider failure recorded by `forward` wins because it is recorded first and the UI reads the latest typed failure — if the vault keeps only the last, skip the local record when `e` came from a provider failure by checking `v.import_jobs()`'s failure for this item before writing).

Rename the module declaration `mod graph_import;` → `mod read_import;` and delete `graph_import.rs`.

- [ ] **Step 3: Remove the tiered extraction code**

In `crates/me-agent/src/graph_pipeline.rs` delete `EXTRACT_STAGE`, `extract_version`, `Line`, `lines`, `extract_state`, `mrz_value`, `period`, `Extraction`, `extract`, `verify_gap_fill` and their tests in `graph_pipeline_tests.rs` (keep classify, search, equivalence and alignment tests; update the live test so it only classifies). In `typesafe_questions.rs` delete `extract_questions`, `verify_questions`, `kind_name`, `MAX_SLOT_OPTIONS`, `MAX_EXTRACT_STATE`, and `PRESENT_ABSENT` if no longer used (keep the ordering `const _` assertion compiling by removing the removed names from it).

- [ ] **Step 4: Build and test the workspace**

Run: `./scripts/cargo build -p me-app`
Expected: builds.

Run: `./scripts/cargo test --workspace`
Expected: PASS.

Run: `./scripts/cargo clippy --workspace --all-targets --all-features -- -D warnings`
Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
./scripts/cargo fmt --all
git add -A apps/desktop/src crates/me-agent
git commit -m "Run one deep-first read pipeline for automatic and manual imports

Every document is classified, read in full with its guide, swept for
omissions, typed locally and verified by TypeSafe before Resolve. The
tiered slot selection and gap fill are removed; reader failures are
never swallowed.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9: "Read from this document" in the document detail

**Files:**
- Modify: `crates/me-core/src/reading.rs` (view model + query), `reading_tests.rs`
- Modify: `apps/desktop/src/shell.rs` (`MeApp.document_read` field), `apps/desktop/src/ai_ui.rs` (`load_proposals` loads it), `apps/desktop/src/vault_ui.rs` (render)
- Modify: `apps/desktop/examples/import_gallery.rs` (synthetic `read` mode)

**Interfaces:**
- Produces:

```rust
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReadRow { pub label: String, pub value: String, pub period: Option<String>, pub location: String, pub uncertain: bool, pub assertion: Option<String>, pub check: bool }
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct DocumentReadView { pub in_profile: Vec<ReadRow>, pub other: Vec<ReadRow>, pub uninterpreted: Vec<ReadRow> }
impl Vault { pub fn document_read(&self, item: u64) -> Result<DocumentReadView> }
```

- [ ] **Step 1: Write the failing test** (append to `reading_tests.rs`)

```rust
#[test]
fn the_document_view_groups_profile_values_other_details_and_uninterpreted_values() {
    let (mut vault, source) = synthetic_payslip_source();
    let item = vault.source_item(&source).unwrap().unwrap();
    let me = vault.profile_entity_id().unwrap();
    let mut uncertain = fact("Kostenstelle", "4711", None);
    uncertain.state = FactState::Uncertain;
    let read = DocumentRead {
        run_id: "r".into(),
        graph: Some(payslip(&me, vec![money("wage_tax", "1032.58", 10), month("2026-01-01", "2026-01-31")])),
        facts: vec![fact("Lohnsteuer", "1.032,58", Some("wage_tax")), uncertain],
        uninterpreted: vec![crate::Uncovered { segment_id: "seg".into(), start: 40, end: 46, kind: CandidateKind::Amount,
            label: Some("KV-Beitrag".into()), text: "435,21".into(), line: "KV-Beitrag 435,21".into() }],
        rejected: Default::default(),
    };
    vault.apply_read(&source, &read).unwrap();
    let view = vault.document_read(item).unwrap();
    assert_eq!(view.in_profile.len(), 1);
    assert_eq!(view.in_profile[0].label, "Lohnsteuer");
    assert!(view.in_profile[0].assertion.is_some());
    assert_eq!(view.other.len(), 1);
    assert!(view.other[0].uncertain);
    assert_eq!(view.uninterpreted[0].value, "435,21");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `./scripts/cargo test -p me-core --lib reading::tests::the_document_view`
Expected: FAIL to compile.

- [ ] **Step 3: Implement the view query** (in `reading.rs`)

```rust
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReadRow {
    pub label: String,
    pub value: String,
    pub period: Option<String>,
    /// Page or section of the source, e.g. "Page 1".
    pub location: String,
    pub uncertain: bool,
    pub assertion: Option<String>,
    /// The linked profile value is waiting in Quick checks.
    pub check: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct DocumentReadView {
    pub in_profile: Vec<ReadRow>,
    pub other: Vec<ReadRow>,
    pub uninterpreted: Vec<ReadRow>,
}

impl Vault {
    pub fn document_read(&self, item: u64) -> Result<DocumentReadView> {
        let mut stmt = self.db.prepare("SELECT f.label,f.value,f.context_quote,coalesce(json_extract(g.locator_json,'$.label'),'Section '||g.ordinal),f.state,f.assertion_id,(SELECT r.check_reason IS NOT NULL FROM assertion_review r WHERE r.assertion_id=f.assertion_id),(SELECT a.state FROM assertion_state a WHERE a.id=f.assertion_id) FROM document_fact f JOIN collection_item i ON i.source_id=f.source_id LEFT JOIN source_segment g ON g.id=json_extract(f.locator_json,'$.segment_id') WHERE i.local_id=? ORDER BY f.ordinal")?;
        let rows = stmt.query_map([crate::vault::sql_id(item)?], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?,
                r.get::<_, String>(4)?, r.get::<_, Option<String>>(5)?, r.get::<_, Option<bool>>(6)?, r.get::<_, Option<String>>(7)?))
        })?;
        let mut view = DocumentReadView::default();
        for row in rows {
            let (label, value, context, location, state, assertion, check, assertion_state) = row?;
            let accepted = assertion_state.as_deref() == Some("accept");
            let entry = ReadRow {
                label, value,
                period: (!context.is_empty()).then_some(context),
                location: location.unwrap_or_default(),
                uncertain: state == "uncertain" || state == "unverified",
                assertion: assertion.filter(|_| accepted),
                check: check.unwrap_or(false) && accepted,
            };
            match state.as_str() {
                "uninterpreted" => view.uninterpreted.push(entry),
                _ if entry.assertion.is_some() => view.in_profile.push(entry),
                _ => view.other.push(entry),
            }
        }
        Ok(view)
    }
}
```

Check the actual segment locator JSON keys in `migrations/001_vault.sql` / `documents.rs` (`source_segment.locator_json`); if it stores a page as `page`, use `coalesce('Page '||json_extract(g.locator_json,'$.page'),'Section '||g.ordinal)` instead of `$.label`. The desktop's existing `Proposal.location_label` computation in `knowledge.rs::proposals` shows the established wording — reuse it.

- [ ] **Step 4: Load and render it**

`shell.rs`: add `pub(super) document_read: Option<me_core::DocumentReadView>,` to `MeApp` (initialize `None` wherever `proposals` is initialized, and clear it where `proposals.clear()` runs).

`ai_ui.rs` `load_proposals`: add `vault.document_read(item)?` to the tuple and assign `this.document_read = Some(read)`.

`vault_ui.rs`: before the legacy `.when(!self.proposals.is_empty(), …)` block, add a section built from `self.document_read`:

```rust
                .when_some(self.document_read.clone().filter(|r| !(r.in_profile.is_empty() && r.other.is_empty() && r.uninterpreted.is_empty())), |s, read| {
                    let group = |title: &'static str, hint: &'static str, rows: Vec<me_core::ReadRow>| {
                        div().flex().flex_col().gap(px(space::SM))
                            .child(div().type_style(Type::Label).font_weight(font::EMPHASIS).child(title))
                            .child(div().type_style(Type::Caption).text_color(rgb(MUTED)).child(hint))
                            .children(rows.into_iter().map(|row| {
                                div().p(px(space::MD)).bg(rgb(BG)).rounded(px(radius::STANDARD)).flex().flex_col().gap(px(space::XS))
                                    .child(div().flex().justify_between().gap(px(space::MD))
                                        .child(div().type_style(Type::Small).min_w_0().child(row.label.clone()))
                                        .child(div().type_style(Type::Small).font_family(font::MONO).flex_shrink_0().child(row.value.clone())))
                                    .child(div().type_style(Type::Caption).text_color(rgb(MUTED)).child(
                                        [Some(row.location.clone()), row.period.clone()].into_iter().flatten().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")))
                                    .when(row.uncertain, |s| s.child(div().type_style(Type::Caption).text_color(rgb(WARNING)).child("Uncertain reading")))
                                    .when(row.check, |s| s.child(div().type_style(Type::Caption).text_color(rgb(ACCENT)).child("Waiting in Quick checks")))
                            }))
                    };
                    s.child(div().type_style(Type::Label).font_weight(font::EMPHASIS).child("Read from this document"))
                        .when(!read.in_profile.is_empty(), |s| s.child(group("In your profile", "Verified values that now appear in your profile and timeline.", read.in_profile.clone())))
                        .when(!read.other.is_empty(), |s| s.child(group("Other details", "Everything else this document states, kept with the document.", read.other.clone())))
                        .when(!read.uninterpreted.is_empty(), |s| s.child(group("Not interpreted", "Printed values no step could explain. Check the original.", read.uninterpreted.clone())))
                })
```

Use only colors that exist in `design_system.rs` (`BG`, `MUTED`, `WARNING`, `ACCENT`); if `WARNING` is not a text color there, use the semantic token the Imports warning text uses (`WARNING` in `import_ui.rs`). Keep the legacy proposals list unchanged for old pending proposals.

- [ ] **Step 5: Synthetic gallery state**

In `apps/desktop/examples/import_gallery.rs` add a `read` mode: create a synthetic personal document (reuse the existing sample import path), call `vault.apply_read(&source, &DocumentRead{…})` with the synthetic payslip facts from `reading_tests.rs`, then set `app.document_open = Some(item)` and `app.document_read = Some(vault.document_read(item).unwrap())` before moving the vault into the session. Follow the file's existing `mode == "…"` pattern and its run instructions at the top of the file.

- [ ] **Step 6: Verify**

Run: `./scripts/cargo test -p me-core --lib reading::tests`
Expected: PASS.

Run: `./scripts/check-design-system && python3 scripts/test-design-system.py && ./scripts/cargo clippy --workspace --all-targets --all-features -- -D warnings`
Expected: PASS.

Run the gallery in `read` mode at 1120×780 and 800×600 (see the header of `import_gallery.rs` for the exact command and window-size flags), capture screenshots, and inspect: three groups, Geist labels, Geist Mono values, 8 px radius, no clipped text at 800×600. Report any state not inspected.

- [ ] **Step 7: Commit**

```bash
./scripts/cargo fmt --all
git add crates/me-core apps/desktop
git commit -m "Show what was read: profile values, other details, not interpreted

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 10: Imports line, Not read yet and Partly read

**Files:**
- Modify: `crates/me-core/src/imports.rs` (`ImportJob.read`), `crates/me-core/tests/import_recovery.rs` or `imports` tests
- Modify: `apps/desktop/src/import_ui.rs` (`import_row` status)
- Modify: `apps/desktop/examples/import_gallery.rs` (states)

**Interfaces:**
- Produces: `#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)] pub struct ReadCounts { pub values: u32, pub in_profile: u32, pub checks: u32, pub uninterpreted: u32 }` and `pub read: Option<ReadCounts>` on `ImportJob`; `pub fn read_status(job: &ImportJob, running: bool) -> Option<String>` in `import_ui.rs` (pure, unit-tested).

- [ ] **Step 1: Write the failing tests**

In `crates/me-core/src/imports.rs` tests (or `tests/import_recovery.rs`, following where `import_jobs` is tested): after `apply_read` on a synthetic source, `vault.import_jobs()` returns `read: Some(ReadCounts { values: 2, in_profile: 1, checks: 0, uninterpreted: 1 })`.

In `apps/desktop/src/import_ui.rs` `mod tests`:

```rust
    fn job(state: &str) -> ImportJob {
        ImportJob { item: 1, title: "x".into(), state: state.into(), stage: ImportStage::Extracting, current: 0, total: 0,
            error: None, warning: None, processable: true, proposals: 0, questions: 0, steps: Default::default(),
            usage: Default::default(), failure: None, read: None }
    }
    #[test]
    fn a_finished_read_reports_counts_and_budget_pauses_say_not_or_partly_read() {
        let mut done = job("done");
        done.read = Some(me_core::ReadCounts { values: 34, in_profile: 12, checks: 1, uninterpreted: 2 });
        assert_eq!(read_status(&done, false).as_deref(), Some("34 values read · 12 in profile · 1 check · 2 not interpreted"));
        let mut paused = job("failed");
        paused.failure = Some(me_core::ImportFailure::new(me_core::ImportProvider::Local, me_core::ImportErrorKind::Budget, "limit"));
        assert_eq!(read_status(&paused, false).as_deref(), Some("Not read yet · Analysis allowance reached"));
        paused.steps[ImportStage::Extracting.index().unwrap()] = me_core::StepProgress { current: 2, total: 5 };
        assert_eq!(read_status(&paused, false).as_deref(), Some("Partly read · Analysis allowance reached"));
        assert_eq!(read_status(&done, true), None);
    }
```

(If `ImportUsage`/`StepProgress` lack `Default`, derive it; both are plain data.)

- [ ] **Step 2: Run to verify failure**

Run: `./scripts/cargo test -p me-app import_ui`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

`imports.rs`: add `ReadCounts`, the `read` field, and extend the `import_jobs` SQL with `LEFT JOIN read_summary rs ON rs.source_id=e.source_id` selecting `rs.values_read, rs.in_profile, rs.checks, rs.uninterpreted` (map to `Some(ReadCounts)` when `values_read` is not NULL).

`import_ui.rs`:

```rust
/// Status line for a finished or budget-paused read; `None` defers to the
/// existing states.
pub(super) fn read_status(job: &ImportJob, running: bool) -> Option<String> {
    if running {
        return None;
    }
    if job.state == "done" && let Some(r) = job.read {
        let plural = |n: u32, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        return Some(format!(
            "{} read · {} in profile · {} · {} not interpreted",
            plural(r.values, "value", "values"),
            r.in_profile,
            plural(r.checks, "check", "checks"),
            r.uninterpreted
        ));
    }
    let budget = job.failure.as_ref().is_some_and(|f| f.kind == me_core::ImportErrorKind::Budget);
    if job.state == "failed" && budget {
        let started = ImportStage::Extracting.index().is_some_and(|i| job.steps[i].current > 0);
        return Some(format!("{} · {}", if started { "Partly read" } else { "Not read yet" }, error_label(me_core::ImportErrorKind::Budget)));
    }
    None
}
```

In `import_row`, compute `let read = read_status(job, running);` and use it first: `let status = if let Some(s) = read { s } else if running { … }` keeping every existing branch after it. Remove the old `"Ready · {} suggestions · {} questions"` branch only for jobs with `job.read.is_some()` (legacy manual reads without a summary keep it).

Gallery: add one finished job with a read summary and one budget-paused job with partial steps to the `progress` mode fixture.

- [ ] **Step 4: Verify**

Run: `./scripts/cargo test -p me-app import_ui && ./scripts/cargo test -p me-core`
Expected: PASS.

Run: `./scripts/check-design-system && python3 scripts/test-design-system.py && ./scripts/cargo clippy --workspace --all-targets --all-features -- -D warnings`
Expected: PASS. Inspect the Imports gallery at 1120×780 and 800×600.

- [ ] **Step 5: Commit**

```bash
./scripts/cargo fmt --all
git add crates/me-core apps/desktop
git commit -m "Report read counts and never show an unread file as done

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 11: Privacy-safe extraction self-check

**Files:**
- Modify: `crates/me-core/src/reading.rs` (+ test in `reading_tests.rs`)
- Modify: `apps/desktop/src/development_ui.rs` and the Settings development section that renders development tools (find the "Wipe data" control; add the button beside it)

**Interfaces:**
- Produces: `impl Vault { pub fn read_self_check(&self) -> Result<String> }` — one line per read document, counts and codes only.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn the_self_check_contains_counts_and_codes_but_no_labels_values_or_titles() {
    let (mut vault, source) = synthetic_payslip_source();
    let me = vault.profile_entity_id().unwrap();
    vault.db.execute("INSERT OR REPLACE INTO document_profile(source_id,family,doc_type,family_confidence,type_confidence,tier,graph_state,classified_at) VALUES(?,'employment','payslip',0.9,0.9,'eager','resolved','2026-09-27')", [&source]).unwrap();
    let read = DocumentRead {
        run_id: "r".into(),
        graph: Some(payslip(&me, vec![money("wage_tax", "1032.58", 10), month("2026-01-01", "2026-01-31")])),
        facts: vec![fact("Lohnsteuer", "1.032,58", Some("wage_tax"))],
        uninterpreted: vec![],
        rejected: [("value_not_in_quote".to_owned(), 2)].into(),
    };
    vault.apply_read(&source, &read).unwrap();
    let report = vault.read_self_check().unwrap();
    assert!(report.contains("employment/payslip"));
    assert!(report.contains("read 1 · profile 1 · checks 0 · not interpreted 0"));
    assert!(report.contains("value_not_in_quote=2"));
    for secret in ["Lohnsteuer", "1.032,58", "1032.58", "payslip.pdf"] {
        assert!(!report.contains(secret), "leaked {secret}");
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `./scripts/cargo test -p me-core --lib reading::tests::the_self_check`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

```rust
impl Vault {
    /// Counts-only diagnosis of every read document, safe to share: no titles,
    /// labels, values or names. Documents are numbered in read order.
    pub fn read_self_check(&self) -> Result<String> {
        let mut stmt = self.db.prepare("SELECT coalesce(p.family,'unclassified'),coalesce(p.doc_type,'-'),r.values_read,r.in_profile,r.checks,r.uninterpreted,r.rejected_json,r.policy FROM read_summary r LEFT JOIN document_profile p ON p.source_id=r.source_id ORDER BY r.recorded_at")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?, r.get::<_, i64>(4)?, r.get::<_, i64>(5)?, r.get::<_, String>(6)?, r.get::<_, String>(7)?)))?;
        let mut out = String::from("ME. extraction self-check (counts only)\n");
        for (n, row) in rows.enumerate() {
            let (family, doc_type, values, profile, checks, open, rejected, policy) = row?;
            let rejected: BTreeMap<String, usize> = serde_json::from_str(&rejected).unwrap_or_default();
            let codes = rejected.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(",");
            out.push_str(&format!("#{} {family}/{doc_type} · read {values} · profile {profile} · checks {checks} · not interpreted {open} · rejected {} · {policy}\n", n + 1, if codes.is_empty() { "none".into() } else { codes }));
        }
        Ok(out)
    }
}
```

`development_ui.rs`: add

```rust
    pub(super) fn copy_read_self_check(&mut self, cx: &mut Context<Self>) {
        let session = self.session.clone();
        let task = cx.background_executor().spawn(async move {
            session.lock().map_err(|_| me_core::Error::Format)?.as_ref().ok_or(me_core::Error::Validation("Vault locked."))?.read_self_check()
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(report) => {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(report));
                        this.notice = Some("Extraction self-check copied. It contains counts only.".into());
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }
```

Render a secondary action "Copy extraction self-check" in the development tools section (same component and spacing as the neighbouring development action; visible only when development tools are enabled, exactly like "Wipe data"). Check the pinned GPUI API for `ClipboardItem::new_string` / `write_to_clipboard` in the vendored source before using it.

- [ ] **Step 4: Verify and commit**

Run: `./scripts/cargo test -p me-core --lib reading::tests && ./scripts/check-design-system && ./scripts/cargo clippy --workspace --all-targets --all-features -- -D warnings`
Expected: PASS.

```bash
./scripts/cargo fmt --all
git add crates/me-core apps/desktop
git commit -m "Add a counts-only extraction self-check to development tools

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 12: Corpus, live evaluation, documentation and final gates

**Files:**
- Create: `crates/me-core/fixtures/documents/{payslip_datev_columns,payslip_sap,payslip_personio_euro,steuerbescheid_refund,steuerbescheid_payment,insurance_letter,bank_statement,invoice,passport_mrz}.txt`
- Create: `crates/me-core/fixtures/documents/expectations.json`
- Create: `crates/me-core/tests/corpus.rs`
- Create: `crates/me-agent/src/live_eval_tests.rs` (wired in `lib.rs` under `#[cfg(test)]`)
- Modify: `docs/import-pipeline.md`, `docs/superpowers/specs/2026-09-27-deep-first-extraction-design.md` (status line), `docs/development.md` (self-check)

**Interfaces:**
- Consumes: `find_candidates`, `locate`, `type_value`, `TypingContext`, `uncovered`, `doc_type`.

- [ ] **Step 1: Write the corpus fixtures**

All values invented; names are `Max Beispiel`/`Erika Beispiel`; organizations `Muster …`. Each file reproduces a real layout:
- `payslip_datev_columns.txt`: the DATEV page from Task 1 as a split-column PDF text layer (labels block first, then the amounts block in the same order), plus the OCR rows appended as a separate block to mirror the macOS OCR pass.
- `payslip_sap.txt`: SAP-style "Entgeltnachweis" with `Lohnart | Text | Betrag` rows, `Summe Brutto`, `Steuerrechtliche Abzüge` (`Lohnsteuer`, `Kirchensteuer rk`, `Solidaritätszuschlag`), `SV-rechtliche Abzüge` (`KV-Beitrag AN`, `RV-Beitrag AN`, `AV-Beitrag AN`, `PV-Beitrag AN`), `Nettobetrag`, `Auszahlung`, `Abrechnungsmonat 02/2026`, `EUR` only in the column header.
- `payslip_personio_euro.txt`: `€` after every amount, `Gehaltsabrechnung März 2026`.
- `steuerbescheid_refund.txt`: `Bescheid für 2025 über Einkommensteuer`, `Veranlagungszeitraum 2025`, `zu versteuerndes Einkommen 48.211`, `festgesetzt werden: Einkommensteuer 7.412,00 €`, `abzüglich Steuerabzug vom Lohn 8.224,00 €`, `Erstattung 812,00 €`, `Vorauszahlungen für 2026 … 0,00 €`.
- `steuerbescheid_payment.txt`: same shape with `Nachzahlung 1.204,00 €` and `fällig am 15.07.2026`.
- `insurance_letter.txt`: sender insurer, recipient, insured person different from policyholder, `Versicherungsschein-Nr.`, premium `monatlich 34,90 €`, `Beginn 01.03.2026`.
- `bank_statement.txt`: account holder IBAN (valid checksum), balance lines `alter Kontostand`/`neuer Kontostand`, three transactions with counterparties' IBANs.
- `invoice.txt`: `Rechnungsnummer`, `Rechnungsdatum`, `Nettobetrag`, `USt 19 %`, `Gesamtbetrag`, `zahlbar bis`, payee IBAN.
- `passport_mrz.txt`: printed fields plus a TD3 MRZ with valid check digits (reuse the synthetic MRZ from `candidates.rs` tests).

`expectations.json`: an object per fixture:

```json
{
  "payslip_datev_rows.txt": {
    "doc_type": "payslip",
    "facts": [
      {"label": "Gesamt-Brutto", "value": "5.340,00", "quote": "Gesamt-Brutto   5.340,00", "slot": "gross", "period": "document", "owner": "self"},
      {"label": "Lohnsteuer", "value": "1.032,58", "quote": "Lohnsteuer   1.032,58", "slot": "wage_tax", "period": "document", "owner": "self"},
      {"label": "Jahreswerte Lohnsteuer", "value": "1.032,58", "quote": "Lohnsteuer   1.032,58", "slot": null, "period": "cumulative", "owner": "self"}
    ]
  }
}
```

List every value worth reading in each fixture (every labeled amount, labeled date and checksum identifier), with its expected slot or `null`. The quote must be an exact substring of the fixture (for the duplicated `Lohnsteuer   1.032,58`, use the full `Jahreswerte …` line as the quote of the cumulative fact).

- [ ] **Step 2: Write the deterministic corpus test** (`crates/me-core/tests/corpus.rs`)

```rust
use me_core::{SourceSegment, TypingContext, doc_type, find_candidates, locate, type_value, uncovered};
use serde_json::Value;

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{}/fixtures/documents/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn every_fixture_value_worth_reading_is_expected_and_every_slot_value_types() {
    let expectations: Value = serde_json::from_str(&fixture("expectations.json")).unwrap();
    for (name, spec) in expectations.as_object().unwrap() {
        let text = fixture(name);
        let segments = [SourceSegment { id: "s0", text: &text }];
        let kind = spec["doc_type"].as_str().and_then(doc_type);
        let ctx = TypingContext::new(&segments, spec["doc_type"].as_str(), 2026);
        let mut covered = Vec::new();
        for f in spec["facts"].as_array().unwrap() {
            let (value, quote) = (f["value"].as_str().unwrap(), f["quote"].as_str().unwrap());
            let at = locate(&segments, "s0", quote, value).unwrap_or_else(|| panic!("{name}: {quote} not verbatim"));
            covered.push(("s0".to_owned(), at.quote_start, at.quote_end));
            if let (Some(k), Some(slot)) = (kind, f["slot"].as_str()) {
                let slot = k.slot(slot).unwrap_or_else(|| panic!("{name}: unknown slot {slot}"));
                assert!(type_value(&ctx, &at, f["label"].as_str().unwrap(), value, slot.value).is_some(),
                    "{name}: {value} does not type as {}", slot.key);
            }
        }
        let open = uncovered(&find_candidates(&segments, &["Max Beispiel"], 2026), &covered);
        assert!(open.is_empty(), "{name}: expectations miss {:?}", open.iter().map(|u| (&u.label, &u.text)).collect::<Vec<_>>());
    }
}
```

Run: `./scripts/cargo test -p me-core --test corpus`
Expected: PASS once `expectations.json` lists every value worth reading. A failure names the missing value; add it to the expectations (never delete fixture content to make it pass).

- [ ] **Step 3: Write the opt-in live evaluation** (`crates/me-agent/src/live_eval_tests.rs`)

Follow the existing opt-in pattern in `graph_pipeline_tests.rs::live_synthetic_payslip_is_classified_and_its_values_selected` (read it for how it gates on the environment and builds `TypeSafe::configured()`), and the Codex live tests in `codex_setup_tests.rs` for the reader home (`ME_LIVE_CODEX_HOME`). The test is `#[ignore]` and, for every fixture in `expectations.json`: builds an `ExtractionInput` from the fixture text split into 1,600-byte segments; classifies; reads with `reading_guide`; types; verifies with `verify_facts` (subject = a synthetic anchor "Max Beispiel"); then compares mapped slots and document facts with the expectations. It prints per family (synthetic data only):

```text
family employment: slot recall 0.00–1.00, slot precision, owner accuracy, period accuracy, false completion (required slot missing but read reported), checks per document, reader requests, TypeSafe requests
```

and asserts only that the run completed (numbers are a baseline, not a gate).

Run (manual, needs ChatGPT + TypeSafe): `./scripts/cargo test -p me-agent live_eval -- --ignored --nocapture`

- [ ] **Step 4: Update documentation**

`docs/import-pipeline.md`: replace the "Tiered pipeline and personal graph" section with "Deep-first reading" describing the implemented pipeline (steps from the spec), the verification bands, the read store, re-reading, the self-check, and a "Verification — <date>" list with exactly what was run: fmt, Clippy, design guard, workspace tests with counts, gallery states inspected, and the live evaluation baseline if it was run. Keep "Not verified" honest: live accuracy on the user's real documents, OS drops, Linux rendering, screen readers, and — if not run — the live evaluation.

Spec status line: `Status: implemented 27 September 2026; see import-pipeline.md for verified behavior and limits.`

`docs/development.md`: add how to copy the extraction self-check from development tools and that it contains counts only.

- [ ] **Step 5: Final gates**

```bash
./scripts/cargo fmt --all -- --check
./scripts/check-design-system
python3 scripts/test-design-system.py
./scripts/cargo clippy --workspace --all-targets --all-features -- -D warnings
./scripts/cargo test --workspace
```

Expected: all PASS. Record the test counts in `docs/import-pipeline.md`.

- [ ] **Step 6: Commit**

```bash
git add crates/me-core/fixtures crates/me-core/tests/corpus.rs crates/me-agent docs
git commit -m "Add the synthetic document corpus, live evaluation and docs

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## Self-Review Notes

- Spec coverage: pipeline steps 1–8 (Tasks 1, 3, 4, 6, 7, 8), verification bands (7), guides incl. verified LStB lines (6), profile values and vocabulary (2), currency and periods (1–3), Resolve changes (5), persistence (5), UI (9, 10), re-reading (5 migration + 8 cache keys), errors and budget (8, 10), testing corpus, live eval, self-check, visual checks and gates (9–12).
- Deviations recorded in the spec before this plan: `document_fact` table instead of `observation` rows; Quick checks only for profile values, uncertain marker for document facts.
- Out of scope per spec: layout geometry, monthly-vs-annual arithmetic, other jurisdictions.
