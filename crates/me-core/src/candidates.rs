//! Local, verbatim-traceable value candidates for document extraction.
//!
//! Imported documents (mostly German personal paperwork: passports, ID cards,
//! tax certificates, payslips, insurance policies, bank letters and vehicle
//! registrations) arrive here as OCR'd or parsed text segments. Small,
//! hand-written scanners propose typed *candidates* such as machine-readable
//! zones, IBANs, tax IDs, dates, amounts, identifiers, names and labeled
//! values.
//!
//! The design is "select instead of generate": a cheap classifier later picks,
//! per field slot, which candidate id holds the right value (or "none"). It can
//! never invent a value, so every candidate carries the exact byte span it was
//! read from (`text == segment.text[start..end]`) and a normalized value that is
//! derived only from that span. Recall matters, but values stay strictly typed:
//! dates are complete `YYYY-MM-DD` values (two-digit years are never expanded,
//! except MRZ dates, which follow the ICAO century rules below), money needs
//! an explicit currency; bare two-decimal amounts become `Amount` candidates
//! whose currency is resolved later, and `checksum` is only set when a real
//! check-digit algorithm validated the value.
//!
//! Every scanner works line by line in linear or locally bounded time, only
//! slices on char boundaries and never panics on arbitrary UTF-8 input.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    Mrz,
    Iban,
    Bic,
    TaxId,
    SocialInsuranceNumber,
    Date,
    Period,
    Money,
    Amount,
    Identifier,
    PersonName,
    Plate,
    Vin,
    Email,
    Phone,
    /// Reserved for the document-type registry (organization and address slots);
    /// `find_candidates` does not emit these kinds yet.
    Organization,
    Street,
    PostalCode,
    City,
    LabelValue,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum CandidateValue {
    Text(String),
    /// Compact normalized identifier without spaces, uppercase where appropriate.
    Identifier(String),
    /// Complete calendar date as `YYYY-MM-DD`.
    Date(String),
    /// Exact decimal amount (`-1234.56`) and ISO-4217 currency code.
    Money {
        amount: String,
        currency: String,
    },
    /// Exact decimal amount printed without a currency marker (`-1234.56`). The
    /// currency is resolved later from the document or its type, never here.
    Amount(String),
    /// A month or year the document names, as its first and last day (inclusive).
    Period {
        start: String,
        end: String,
    },
    Mrz(MrzData),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MrzData {
    /// `td1`, `td2` or `td3`.
    pub format: String,
    pub document_code: String,
    pub issuing_state: String,
    pub surname: String,
    pub given_names: String,
    pub document_number: String,
    pub nationality: String,
    pub birth_date: Option<String>,
    pub sex: String,
    pub expiry_date: Option<String>,
    /// All check digits, including the composite check digit, are valid.
    pub check_digits_valid: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    /// `c0`, `c1`, ... in document order.
    pub id: String,
    pub kind: CandidateKind,
    /// Exactly `segment.text[start..end]`.
    pub text: String,
    pub value: CandidateValue,
    pub segment_id: String,
    /// Byte offsets into the segment text, on char boundaries.
    pub start: usize,
    pub end: usize,
    /// The trimmed source line (at most 240 bytes, windowed around the candidate).
    pub line: String,
    /// Nearest printed label, at most 80 bytes.
    pub label: Option<String>,
    /// True only when an actual checksum validated the value.
    pub checksum: bool,
}

pub struct SourceSegment<'a> {
    pub id: &'a str,
    pub text: &'a str,
}

/// Upper bound so one TypeSafe Choice (max 255 options) can include all candidates plus "none".
pub const MAX_CANDIDATES: usize = 200;

const LINE_MAX: usize = 240;
pub(crate) const LABEL_MAX: usize = 80;
/// How far before a candidate a same-line `Label:` is searched (bounds work on huge lines).
const LABEL_SCAN: usize = 256;
const PREV_LABEL_MAX: usize = 40;

/// Finds typed, verbatim-traceable value candidates in the given segments.
///
/// `known_names`: names/aliases of the user and household members, used to emit
/// `PersonName` candidates for their occurrences (case-insensitive, `ä/ö/ü/ß` ↔
/// `ae/oe/ue/ss`, "Surname, Given" and MRZ-like "SURNAME GIVEN" orders).
/// `reference_year`: current year, used only for MRZ two-digit-year century decisions.
pub fn find_candidates(
    segments: &[SourceSegment<'_>],
    known_names: &[&str],
    reference_year: i32,
) -> Vec<Candidate> {
    let names = NameMatcher::new(known_names);
    let docs: Vec<Doc<'_>> = segments.iter().map(|s| Doc::new(s.text)).collect();
    let mut pool = Vec::new();
    for (seg, doc) in docs.iter().enumerate() {
        for raw in scan_document(doc, &names, reference_year) {
            let label = raw.label.or_else(|| doc.label_for(raw.start));
            pool.push(Pooled { seg, raw, label });
        }
    }
    select(pool)
        .into_iter()
        .enumerate()
        .map(|(n, pooled)| {
            let doc = &docs[pooled.seg];
            let Raw {
                kind,
                start,
                end,
                value,
                checksum,
                ..
            } = pooled.raw;
            Candidate {
                id: format!("c{n}"),
                kind,
                text: doc.text[start..end].to_string(),
                value,
                segment_id: segments[pooled.seg].id.to_string(),
                start,
                end,
                line: doc.line_window(start, end),
                label: pooled
                    .label
                    .map(|(a, b)| truncate_bytes(&doc.text[a..b], LABEL_MAX).to_string()),
                checksum,
            }
        })
        .collect()
}

/// A candidate before ids, lines and labels are materialized. Offsets are
/// absolute within the segment text.
struct Raw {
    kind: CandidateKind,
    start: usize,
    end: usize,
    value: CandidateValue,
    checksum: bool,
    /// Explicit label span; otherwise the label is detected around `start`.
    label: Option<(usize, usize)>,
}

struct Pooled {
    seg: usize,
    raw: Raw,
    label: Option<(usize, usize)>,
}

fn kind_priority(kind: CandidateKind) -> u8 {
    use CandidateKind::*;
    match kind {
        Mrz => 0,
        Iban => 1,
        TaxId => 2,
        SocialInsuranceNumber => 3,
        Vin => 4,
        Plate => 5,
        Identifier => 6,
        Date => 7,
        Money => 8,
        Period => 7,
        Amount => 8,
        PersonName => 9,
        Bic => 10,
        Email => 11,
        Phone => 12,
        Organization | Street | PostalCode | City => 13,
        LabelValue => 14,
    }
}

/// Keeps at most [`MAX_CANDIDATES`], preferring checksum-validated, labeled and
/// more specific candidates, and returns them in document order.
fn select(pool: Vec<Pooled>) -> Vec<Pooled> {
    if pool.len() <= MAX_CANDIDATES {
        return pool;
    }
    let mut order: Vec<usize> = (0..pool.len()).collect();
    order.sort_by_key(|&i| {
        let p = &pool[i];
        (
            !p.raw.checksum,
            p.label.is_none(),
            kind_priority(p.raw.kind),
            i,
        )
    });
    let mut keep = vec![false; pool.len()];
    for &i in order.iter().take(MAX_CANDIDATES) {
        keep[i] = true;
    }
    pool.into_iter()
        .zip(keep)
        .filter_map(|(p, k)| k.then_some(p))
        .collect()
}

fn scan_document(doc: &Doc<'_>, names: &NameMatcher, reference_year: i32) -> Vec<Raw> {
    let mut out = Vec::new();
    scan_mrz(doc, reference_year, &mut out);
    for index in 0..doc.lines.len() {
        let Some(ctx) = LineCtx::new(doc, index) else {
            continue;
        };
        // Every scanner in this block needs a digit somewhere on the line.
        if ctx.text.bytes().any(|b| b.is_ascii_digit()) {
            scan_iban(&ctx, &mut out);
            scan_tax_id(&ctx, &mut out);
            scan_social_insurance(&ctx, &mut out);
            scan_dates(&ctx, &mut out);
            scan_periods(&ctx, &mut out);
            scan_money(&ctx, &mut out);
            scan_amounts(&ctx, &mut out);
            scan_identifiers(&ctx, &mut out);
            scan_plates(&ctx, &mut out);
            scan_vin(&ctx, &mut out);
            scan_phone(&ctx, &mut out);
        }
        if ctx.text.contains('@') {
            scan_email(&ctx, &mut out);
        }
        scan_bic(&ctx, &mut out);
        scan_organizations(&ctx, &mut out);
        scan_address(&ctx, &mut out);
        names.scan(&ctx, &mut out);
        scan_name_triggers(&ctx, &mut out);
        scan_label_values(&ctx, &mut out);
    }
    clean_up(out)
}

/// Sorts into document order, removes duplicates and redundant catch-alls.
/// A label/value pair that duplicates a more specific candidate hands its label
/// to that candidate instead (e.g. `Versicherungsbeginn    01.02.2024`).
fn clean_up(mut raws: Vec<Raw>) -> Vec<Raw> {
    use CandidateKind::*;
    raws.sort_by_key(|r| (r.start, r.kind, r.end, !r.checksum, r.label.is_none()));
    raws.dedup_by(|later, kept| {
        later.start == kept.start && later.end == kept.end && later.kind == kept.kind
    });
    let ibans: Vec<(usize, usize)> = raws
        .iter()
        .filter(|r| r.kind == Iban)
        .map(|r| (r.start, r.end))
        .collect();
    let specific: HashSet<(usize, usize)> = raws
        .iter()
        .filter(|r| !matches!(r.kind, LabelValue | Identifier | PersonName))
        .map(|r| (r.start, r.end))
        .collect();
    let any: HashSet<(usize, usize)> = raws
        .iter()
        .filter(|r| r.kind != LabelValue)
        .map(|r| (r.start, r.end))
        .collect();
    let pair_labels: HashMap<(usize, usize), (usize, usize)> = raws
        .iter()
        .filter(|r| r.kind == LabelValue)
        .filter_map(|r| Some(((r.start, r.end), r.label?)))
        .collect();
    for raw in raws
        .iter_mut()
        .filter(|r| r.kind != LabelValue && r.label.is_none())
    {
        raw.label = pair_labels.get(&(raw.start, raw.end)).copied();
    }
    raws.retain(|r| {
        let span = (r.start, r.end);
        match r.kind {
            LabelValue => !any.contains(&span),
            Identifier if !r.checksum => !specific.contains(&span) && !overlaps_any(&ibans, span),
            Phone | TaxId | SocialInsuranceNumber | Plate => !overlaps_any(&ibans, span),
            _ => true,
        }
    });
    raws
}

/// `spans` is sorted by start and non-overlapping.
fn overlaps_any(spans: &[(usize, usize)], (start, end): (usize, usize)) -> bool {
    let i = spans.partition_point(|s| s.0 < end);
    i > 0 && spans[i - 1].1 > start
}

/// One segment split into lines, with the helpers shared by all scanners.
struct Doc<'a> {
    text: &'a str,
    /// Line content ranges without the line break (`\n` or `\r\n`).
    lines: Vec<(usize, usize)>,
    prev_nonempty: Vec<Option<usize>>,
    /// Whether any line carries an `EUR`/`Ct` table header (see [`has_euro_cent_columns`]).
    euro_cent: bool,
}

impl<'a> Doc<'a> {
    fn new(text: &'a str) -> Self {
        let bytes = text.as_bytes();
        let mut lines = Vec::new();
        let mut start = 0;
        for (i, &byte) in bytes.iter().enumerate() {
            if byte == b'\n' {
                let end = if i > start && bytes[i - 1] == b'\r' {
                    i - 1
                } else {
                    i
                };
                lines.push((start, end));
                start = i + 1;
            }
        }
        lines.push((start, text.len()));
        let mut prev_nonempty = Vec::with_capacity(lines.len());
        let mut last = None;
        for (i, &(s, e)) in lines.iter().enumerate() {
            prev_nonempty.push(last);
            if !text[s..e].trim().is_empty() {
                last = Some(i);
            }
        }
        let euro_cent = has_euro_cent_columns(text);
        Self {
            text,
            lines,
            prev_nonempty,
            euro_cent,
        }
    }

    fn line_str(&self, index: usize) -> &'a str {
        let (s, e) = self.lines[index];
        &self.text[s..e]
    }

    fn line_of(&self, pos: usize) -> usize {
        self.lines
            .partition_point(|&(s, _)| s <= pos)
            .saturating_sub(1)
    }

    /// Nearest printed label for a candidate starting at `pos`: `Label:` before it
    /// on the same line, otherwise a short previous line that ends with `:` or has
    /// no digits (only when nothing textual precedes the candidate on its line).
    fn label_for(&self, pos: usize) -> Option<(usize, usize)> {
        let index = self.line_of(pos);
        let line_start = self.lines[index].0;
        if let Some(span) = same_line_label(self.text, line_start, pos) {
            return Some(span);
        }
        if pos - line_start > LABEL_SCAN || has_letter(&self.text[line_start..pos]) {
            return None;
        }
        let prev = self.prev_nonempty[index]?;
        let (ps, pe) = trim_span(self.text, self.lines[prev]);
        if pe - ps > PREV_LABEL_MAX {
            return None;
        }
        let line = &self.text[ps..pe];
        let end = if line.ends_with(':') {
            pe - 1
        } else if line.bytes().any(|b| b.is_ascii_digit()) {
            return None;
        } else {
            pe
        };
        let (ls, le) = trim_span(self.text, (ps, end));
        (le > ls && has_letter(&self.text[ls..le])).then_some((ls, le))
    }

    /// The trimmed line(s) containing `start..end`, windowed to [`LINE_MAX`] bytes.
    fn line_window(&self, start: usize, end: usize) -> String {
        let first = self.lines[self.line_of(start)].0;
        let last = self.lines[self.line_of(end.saturating_sub(1).max(start))].1;
        let (ts, te) = trim_span(self.text, (first, last.max(end)));
        if te - ts <= LINE_MAX {
            return self.text[ts..te].to_string();
        }
        let len = end - start;
        let (ws, we) = if len >= LINE_MAX {
            (start, start + LINE_MAX)
        } else {
            let pad = (LINE_MAX - len) / 2;
            let ws = start.saturating_sub(pad).max(ts);
            let we = (ws + LINE_MAX).min(te);
            (we.saturating_sub(LINE_MAX).max(ts), we)
        };
        let ws = ceil_boundary(self.text, ws);
        let we = floor_boundary(self.text, we);
        if ws >= we {
            return String::new();
        }
        self.text[ws..we].trim().to_string()
    }
}

/// The trimmed line(s) of `text` containing the value span `start..end`, windowed
/// to [`LINE_MAX`] bytes around the value (same rule as [`Doc::line_window`], for
/// callers that only have a segment's text and a value span, not a `Doc`).
pub(crate) fn line_window_for(text: &str, start: usize, end: usize) -> String {
    Doc::new(text).line_window(start, end)
}

/// `Label:` before `pos` on the same line (searched within [`LABEL_SCAN`] bytes).
fn same_line_label(text: &str, line_start: usize, pos: usize) -> Option<(usize, usize)> {
    let from = ceil_boundary(text, pos.saturating_sub(LABEL_SCAN).max(line_start));
    if from >= pos {
        return None;
    }
    let bytes = &text.as_bytes()[from..pos];
    let colon = (0..bytes.len()).rev().find(|&i| {
        bytes[i] == b':'
            && !(i > 0 && bytes[i - 1].is_ascii_digit())
            && bytes.get(i + 1) != Some(&b'/')
    })?;
    let mut label_start = 0;
    let mut i = colon;
    while i > 0 {
        let b = bytes[i - 1];
        if matches!(b, b':' | b'\t' | b'|' | b';') || (b == b' ' && i >= 2 && bytes[i - 2] == b' ')
        {
            label_start = i;
            break;
        }
        i -= 1;
    }
    let (mut ls, le) = trim_span(text, (from + label_start, from + colon));
    if le - ls > LABEL_MAX {
        ls = ceil_boundary(text, le - LABEL_MAX);
        if let Some(space) = text[ls..le].find(' ') {
            ls += space + 1;
        }
    }
    (le > ls && has_letter(&text[ls..le])).then_some((ls, le))
}

/// Per-line scanning context.
struct LineCtx<'a> {
    doc: &'a Doc<'a>,
    index: usize,
    /// Absolute offset of the line start in the segment.
    base: usize,
    text: &'a str,
    /// ASCII-lowercased copy with identical byte offsets.
    lower: String,
}

impl<'a> LineCtx<'a> {
    fn new(doc: &'a Doc<'a>, index: usize) -> Option<Self> {
        let text = doc.line_str(index);
        if text.trim().is_empty() {
            return None;
        }
        Some(Self {
            doc,
            index,
            base: doc.lines[index].0,
            text,
            lower: text.to_ascii_lowercase(),
        })
    }

    /// A raw candidate for the line-local span `start..end`.
    fn raw(&self, kind: CandidateKind, start: usize, end: usize, value: CandidateValue) -> Raw {
        Raw {
            kind,
            start: self.base + start,
            end: self.base + end,
            value,
            checksum: false,
            label: None,
        }
    }

    fn abs(&self, (start, end): (usize, usize)) -> (usize, usize) {
        (self.base + start, self.base + end)
    }

    /// Detected label text for a candidate starting at line-local `pos`.
    fn label(&self, pos: usize) -> Option<&'a str> {
        self.doc
            .label_for(self.base + pos)
            .map(|(a, b)| &self.doc.text[a..b])
    }

    /// Previous non-empty line, trimmed (empty when there is none).
    fn prev_line(&self) -> &'a str {
        self.doc.prev_nonempty[self.index]
            .map(|i| self.doc.line_str(i).trim())
            .unwrap_or("")
    }

    /// Whether the label or the text shortly before `pos` mentions any keyword
    /// (`keywords` must be lowercase ASCII).
    /// Whether this or the previous line mentions any keyword at all (a cheap,
    /// once-per-line precondition for [`Self::near_keyword`]).
    fn mentions(&self, keywords: &[&str]) -> bool {
        contains_any(&self.lower, keywords)
            || contains_any(&self.prev_line().to_ascii_lowercase(), keywords)
    }

    fn near_keyword(&self, pos: usize, keywords: &[&str]) -> bool {
        let from = floor_boundary(self.text, pos.saturating_sub(60));
        contains_any(&self.lower[from..pos], keywords)
            || self
                .label(pos)
                .is_some_and(|label| contains_any(&label.to_ascii_lowercase(), keywords))
    }
}

fn prev_char(s: &str, i: usize) -> Option<char> {
    s.get(..i)?.chars().next_back()
}

fn next_char(s: &str, i: usize) -> Option<char> {
    s.get(i..)?.chars().next()
}

/// No alphanumeric character directly before `i`.
fn word_start(s: &str, i: usize) -> bool {
    prev_char(s, i).is_none_or(|c| !c.is_alphanumeric())
}

/// No alphanumeric character at `i`.
fn word_end(s: &str, i: usize) -> bool {
    next_char(s, i).is_none_or(|c| !c.is_alphanumeric())
}

fn is_number_joiner(b: u8) -> bool {
    matches!(b, b'.' | b',' | b'/' | b'-' | b'\'')
}

/// A number may start at `i`: not glued to a word or to a preceding number such
/// as the `234` in `1.234` or the `02` in `01/02`.
fn number_start(s: &str, i: usize) -> bool {
    let b = s.as_bytes();
    word_start(s, i) && !(i >= 2 && is_number_joiner(b[i - 1]) && b[i - 2].is_ascii_digit())
}

/// A number may end at `i` (mirror of [`number_start`]).
fn number_end(s: &str, i: usize) -> bool {
    let b = s.as_bytes();
    word_end(s, i) && !(i + 1 < b.len() && is_number_joiner(b[i]) && b[i + 1].is_ascii_digit())
}

fn floor_boundary(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_boundary(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

pub(crate) fn truncate_bytes(s: &str, max: usize) -> &str {
    s[..floor_boundary(s, max)].trim_end()
}

fn trim_span(s: &str, (a, b): (usize, usize)) -> (usize, usize) {
    let part = &s[a..b];
    let start = a + (part.len() - part.trim_start().len());
    (start, start + part.trim().len())
}

fn has_letter(s: &str) -> bool {
    s.chars().any(char::is_alphabetic)
}

fn contains_any(hay: &str, needles: &[&str]) -> bool {
    needles
        .iter()
        .any(|n| find_bytes(hay.as_bytes(), n.as_bytes(), 0).is_some())
}

/// Plain substring search from `from`. Cheaper than `str::find` for the many
/// short per-line keyword checks (no searcher setup); needles are short.
fn find_bytes(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    let (&first, rest) = needle.split_first()?;
    let last_start = hay.len().checked_sub(needle.len())?;
    (from..=last_start).find(|&i| hay[i] == first && hay[i + 1..i + needle.len()] == *rest)
}

/// Non-overlapping occurrences of `needle` (valid UTF-8, so every match is on a
/// char boundary).
fn occurrences<'h>(hay: &'h str, needle: &'h str) -> impl Iterator<Item = usize> + 'h {
    let mut from = 0;
    std::iter::from_fn(move || {
        let found = find_bytes(hay.as_bytes(), needle.as_bytes(), from)?;
        from = found + needle.len().max(1);
        Some(found)
    })
}

fn digit_count(s: &str) -> usize {
    s.bytes().filter(u8::is_ascii_digit).count()
}

/// Maximal runs of alphanumeric characters as byte spans.
fn word_tokens(s: &str) -> Vec<(usize, usize)> {
    let mut tokens = Vec::new();
    let mut start = None;
    for (i, c) in s.char_indices() {
        match (c.is_alphanumeric(), start) {
            (true, None) => start = Some(i),
            (false, Some(st)) => {
                tokens.push((st, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(st) = start {
        tokens.push((st, s.len()));
    }
    tokens
}

fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Formats a validated calendar date as `YYYY-MM-DD`.
fn iso_date(year: i32, month: u32, day: u32) -> Option<String> {
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => return None,
    };
    (day >= 1 && day <= days && (0..=9999).contains(&year))
        .then(|| format!("{year:04}-{month:02}-{day:02}"))
}

// ---------------------------------------------------------------------------
// MRZ (ICAO 9303: TD1 3×30, TD2 2×36, TD3 2×44)
// ---------------------------------------------------------------------------

/// A normalized MRZ line with the source span of every normalized character.
struct MrzLine {
    chars: Vec<u8>,
    starts: Vec<usize>,
    ends: Vec<usize>,
}

impl MrzLine {
    /// Normalizes OCR noise: spaces removed, uppercase, `«`/`‹` → `<`.
    fn parse(doc: &Doc<'_>, index: usize) -> Option<Self> {
        let (line_start, _) = doc.lines[index];
        let line = doc.line_str(index);
        let trimmed = line.trim();
        if !(30..=140).contains(&trimmed.len()) {
            return None;
        }
        let offset = line_start + (line.len() - line.trim_start().len());
        let mut out = Self {
            chars: Vec::with_capacity(44),
            starts: Vec::with_capacity(44),
            ends: Vec::with_capacity(44),
        };
        for (i, c) in trimmed.char_indices() {
            let normalized = match c {
                ' ' | '\t' => continue,
                '<' | '«' | '‹' => b'<',
                c if c.is_ascii_alphanumeric() => c.to_ascii_uppercase() as u8,
                _ => return None,
            };
            out.chars.push(normalized);
            out.starts.push(offset + i);
            out.ends.push(offset + i + c.len_utf8());
        }
        matches!(out.chars.len(), 30 | 36 | 44).then_some(out)
    }

    fn len(&self) -> usize {
        self.chars.len()
    }

    /// Absolute source span of normalized characters `a..b` (`a < b`).
    fn span(&self, a: usize, b: usize) -> (usize, usize) {
        (self.starts[a], self.ends[b - 1])
    }
}

fn mrz_value(c: u8) -> u32 {
    match c {
        b'0'..=b'9' => u32::from(c - b'0'),
        b'A'..=b'Z' => u32::from(c - b'A') + 10,
        _ => 0,
    }
}

/// ICAO 9303 check digit (weights 7, 3, 1; `<` = 0, `A` = 10 … `Z` = 35).
fn mrz_check_digit<'b>(data: impl IntoIterator<Item = &'b u8>) -> u32 {
    const WEIGHTS: [u32; 3] = [7, 3, 1];
    data.into_iter()
        .enumerate()
        .map(|(i, &c)| mrz_value(c) * WEIGHTS[i % 3])
        .sum::<u32>()
        % 10
}

fn mrz_check_ok(data: &[u8], check: u8) -> bool {
    (check.is_ascii_digit() || check == b'<') && mrz_value(check) == mrz_check_digit(data)
}

fn mrz_composite_ok(parts: &[&[u8]], check: u8) -> bool {
    (check.is_ascii_digit() || check == b'<')
        && mrz_value(check) == mrz_check_digit(parts.iter().flat_map(|p| p.iter()))
}

#[derive(Clone, Copy)]
enum MrzCentury {
    Birth,
    Expiry,
}

/// `YYMMDD` → `YYYY-MM-DD`. Birth: `YY > reference_year % 100` → 19YY, else 20YY.
/// Expiry: always 20YY.
fn mrz_date(field: &[u8], century: MrzCentury, reference_year: i32) -> Option<String> {
    if field.len() != 6 || !field.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let num = |i: usize| u32::from(field[i] - b'0') * 10 + u32::from(field[i + 1] - b'0');
    let yy = num(0) as i32;
    let year = match century {
        MrzCentury::Birth if yy > reference_year.rem_euclid(100) => 1900 + yy,
        _ => 2000 + yy,
    };
    iso_date(year, num(2), num(4))
}

fn mrz_text(field: &[u8]) -> String {
    field
        .iter()
        .map(|&c| if c == b'<' { ' ' } else { char::from(c) })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn mrz_names(field: &[u8]) -> (String, String) {
    match field.windows(2).position(|w| w == b"<<") {
        Some(split) => (mrz_text(&field[..split]), mrz_text(&field[split + 2..])),
        None => (mrz_text(field), String::new()),
    }
}

fn is_mrz_alpha(field: &[u8]) -> bool {
    field.iter().all(|&c| c.is_ascii_uppercase() || c == b'<')
}

/// Field positions of one parsed MRZ; ranges refer to normalized characters of
/// `lines[line]`.
struct MrzFields {
    data: MrzData,
    number: (usize, usize, usize),
    number_ok: bool,
    birth: (usize, usize),
    birth_ok: bool,
    expiry: (usize, usize),
    expiry_ok: bool,
}

/// Parses TD2 (2×36) or TD3 (2×44): name line, then data line.
fn parse_td23(l1: &[u8], l2: &[u8], reference_year: i32) -> Option<MrzFields> {
    let width = l1.len();
    if l2.len() != width
        || !l1[0].is_ascii_uppercase()
        || !is_mrz_alpha(&l1[..5])
        || !l1.contains(&b'<')
        || !is_mrz_alpha(&l2[10..13])
        || !matches!(l2[20], b'M' | b'F' | b'X' | b'<')
    {
        return None;
    }
    let number_ok = mrz_check_ok(&l2[0..9], l2[9]);
    let birth_ok = mrz_check_ok(&l2[13..19], l2[19]);
    let expiry_ok = mrz_check_ok(&l2[21..27], l2[27]);
    let last = width - 1;
    let composite_ok = mrz_composite_ok(&[&l2[0..10], &l2[13..20], &l2[21..last]], l2[last]);
    let (format, optional_ok) = if width == 44 {
        ("td3", mrz_check_ok(&l2[28..42], l2[42]))
    } else {
        ("td2", true)
    };
    let (surname, given_names) = mrz_names(&l1[5..]);
    Some(MrzFields {
        data: MrzData {
            format: format.into(),
            document_code: mrz_text(&l1[0..2]).replace(' ', ""),
            issuing_state: mrz_text(&l1[2..5]).replace(' ', ""),
            surname,
            given_names,
            document_number: mrz_text(&l2[0..9]).replace(' ', ""),
            nationality: mrz_text(&l2[10..13]).replace(' ', ""),
            birth_date: mrz_date(&l2[13..19], MrzCentury::Birth, reference_year),
            sex: mrz_sex(l2[20]),
            expiry_date: mrz_date(&l2[21..27], MrzCentury::Expiry, reference_year),
            check_digits_valid: number_ok && birth_ok && expiry_ok && optional_ok && composite_ok,
        },
        number: (1, 0, 9),
        number_ok,
        birth: (13, 19),
        birth_ok,
        expiry: (21, 27),
        expiry_ok,
    })
}

/// Parses TD1: document line, data line, name line.
fn parse_td1(l1: &[u8], l2: &[u8], l3: &[u8], reference_year: i32) -> Option<MrzFields> {
    if l1.len() != 30
        || l2.len() != 30
        || l3.len() != 30
        || !l1[0].is_ascii_uppercase()
        || !is_mrz_alpha(&l1[..5])
        || !is_mrz_alpha(&l2[15..18])
        || !matches!(l2[7], b'M' | b'F' | b'X' | b'<')
        || !l3.contains(&b'<')
        || !is_mrz_alpha(l3)
    {
        return None;
    }
    let number_ok = mrz_check_ok(&l1[5..14], l1[14]);
    let birth_ok = mrz_check_ok(&l2[0..6], l2[6]);
    let expiry_ok = mrz_check_ok(&l2[8..14], l2[14]);
    let composite_ok = mrz_composite_ok(&[&l1[5..30], &l2[0..7], &l2[8..15], &l2[18..29]], l2[29]);
    let (surname, given_names) = mrz_names(l3);
    Some(MrzFields {
        data: MrzData {
            format: "td1".into(),
            document_code: mrz_text(&l1[0..2]).replace(' ', ""),
            issuing_state: mrz_text(&l1[2..5]).replace(' ', ""),
            surname,
            given_names,
            document_number: mrz_text(&l1[5..14]).replace(' ', ""),
            nationality: mrz_text(&l2[15..18]).replace(' ', ""),
            birth_date: mrz_date(&l2[0..6], MrzCentury::Birth, reference_year),
            sex: mrz_sex(l2[7]),
            expiry_date: mrz_date(&l2[8..14], MrzCentury::Expiry, reference_year),
            check_digits_valid: number_ok && birth_ok && expiry_ok && composite_ok,
        },
        number: (0, 5, 14),
        number_ok,
        birth: (0, 6),
        birth_ok,
        expiry: (8, 14),
        expiry_ok,
    })
}

fn mrz_sex(c: u8) -> String {
    match c {
        b'M' => "M",
        b'F' => "F",
        _ => "X",
    }
    .into()
}

/// Finds MRZ blocks on consecutive non-empty lines and emits the MRZ itself plus
/// the document number and dates as separate candidates.
fn scan_mrz(doc: &Doc<'_>, reference_year: i32, out: &mut Vec<Raw>) {
    let nonempty: Vec<usize> = (0..doc.lines.len())
        .filter(|&i| !doc.line_str(i).trim().is_empty())
        .collect();
    let parsed: Vec<Option<MrzLine>> = nonempty.iter().map(|&i| MrzLine::parse(doc, i)).collect();
    let width = |k: usize| parsed.get(k).and_then(Option::as_ref).map(MrzLine::len);
    let mut k = 0;
    while k < parsed.len() {
        let group = match (width(k), width(k + 1), width(k + 2)) {
            (Some(30), Some(30), Some(30)) => 3,
            (Some(36), Some(36), _) | (Some(44), Some(44), _) => 2,
            _ => {
                k += 1;
                continue;
            }
        };
        let lines: Vec<&MrzLine> = parsed[k..k + group].iter().flatten().collect();
        let fields = if group == 3 {
            parse_td1(
                &lines[0].chars,
                &lines[1].chars,
                &lines[2].chars,
                reference_year,
            )
        } else {
            parse_td23(&lines[0].chars, &lines[1].chars, reference_year)
        };
        match fields {
            Some(fields) => {
                emit_mrz(&lines, fields, out);
                k += group;
            }
            None => k += 1,
        }
    }
}

fn emit_mrz(lines: &[&MrzLine], fields: MrzFields, out: &mut Vec<Raw>) {
    let first = lines[0];
    let last = lines[lines.len() - 1];
    let raw = |kind, (start, end): (usize, usize), value, checksum| Raw {
        kind,
        start,
        end,
        value,
        checksum,
        label: None,
    };
    let (line, a, b) = fields.number;
    let number = &lines[line].chars[a..b];
    let used = number.len() - number.iter().rev().take_while(|&&c| c == b'<').count();
    if used > 0 {
        out.push(raw(
            CandidateKind::Identifier,
            lines[line].span(a, a + used),
            CandidateValue::Identifier(fields.data.document_number.clone()),
            fields.number_ok,
        ));
    }
    let date_line = lines[1];
    for (range, ok, date) in [
        (fields.birth, fields.birth_ok, &fields.data.birth_date),
        (fields.expiry, fields.expiry_ok, &fields.data.expiry_date),
    ] {
        if let Some(date) = date {
            out.push(raw(
                CandidateKind::Date,
                date_line.span(range.0, range.1),
                CandidateValue::Date(date.clone()),
                ok,
            ));
        }
    }
    let valid = fields.data.check_digits_valid;
    out.push(raw(
        CandidateKind::Mrz,
        (first.starts[0], last.ends[last.len() - 1]),
        CandidateValue::Mrz(fields.data),
        valid,
    ));
}

// ---------------------------------------------------------------------------
// IBAN and BIC
// ---------------------------------------------------------------------------

fn iban_length(country: &str) -> Option<usize> {
    Some(match country {
        "NO" => 15,
        "BE" => 16,
        "DK" | "FI" | "NL" => 18,
        "SI" => 19,
        "AT" | "EE" | "LT" | "LU" => 20,
        "CH" | "HR" | "LI" | "LV" => 21,
        "BG" | "DE" | "GB" | "IE" => 22,
        "CZ" | "ES" | "RO" | "SE" | "SK" => 24,
        "PT" => 25,
        "IS" => 26,
        "FR" | "GR" | "IT" | "MC" | "SM" => 27,
        "CY" | "HU" | "PL" => 28,
        "MT" => 31,
        _ => return None,
    })
}

/// ISO 13616 mod-97 check on a compact uppercase IBAN.
fn iban_mod97_ok(iban: &str) -> bool {
    let bytes = iban.as_bytes();
    if bytes.len() < 5 {
        return false;
    }
    let mut rest: u32 = 0;
    for &c in bytes[4..].iter().chain(&bytes[..4]) {
        rest = match c {
            b'0'..=b'9' => (rest * 10 + u32::from(c - b'0')) % 97,
            b'A'..=b'Z' => (rest * 100 + u32::from(c - b'A') + 10) % 97,
            _ => return false,
        };
    }
    rest == 1
}

/// Country code + 2 check digits + BBAN, optionally grouped by single spaces.
fn scan_iban(ctx: &LineCtx<'_>, out: &mut Vec<Raw>) {
    let s = ctx.text;
    let b = s.as_bytes();
    let mut i = 0;
    while i + 4 <= b.len() {
        let head = b[i].is_ascii_uppercase()
            && b[i + 1].is_ascii_uppercase()
            && b[i + 2].is_ascii_digit()
            && b[i + 3].is_ascii_digit();
        if head
            && word_start(s, i)
            && let Some((end, iban)) = match_iban(s, i)
        {
            out.push(Raw {
                checksum: true,
                ..ctx.raw(
                    CandidateKind::Iban,
                    i,
                    end,
                    CandidateValue::Identifier(iban),
                )
            });
            i = end;
            continue;
        }
        i += 1;
    }
}

fn match_iban(s: &str, start: usize) -> Option<(usize, String)> {
    let b = s.as_bytes();
    let mut compact = String::with_capacity(34);
    // (byte end, compact length) at every group end.
    let mut group_ends = Vec::new();
    let mut i = start;
    while i < b.len() && compact.len() < 34 {
        let c = b[i];
        if c.is_ascii_alphanumeric() {
            compact.push(char::from(c.to_ascii_uppercase()));
            i += 1;
            if i >= b.len() || !b[i].is_ascii_alphanumeric() {
                group_ends.push((i, compact.len()));
            }
        } else if c == b' ' && b.get(i + 1).is_some_and(u8::is_ascii_alphanumeric) {
            i += 1;
        } else {
            break;
        }
    }
    let accept = |&(end, len): &(usize, usize)| {
        let iban = &compact[..len];
        word_end(s, end) && iban_mod97_ok(iban)
    };
    let found = match iban_length(&compact[..2]) {
        Some(expected) => group_ends
            .iter()
            .find(|&&(_, len)| len == expected)
            .filter(|g| accept(g)),
        None => group_ends
            .iter()
            .rev()
            .filter(|&&(_, len)| (15..=34).contains(&len))
            .find(|g| accept(g)),
    };
    found.map(|&(end, len)| (end, compact[..len].to_string()))
}

/// 8 or 11 character BIC, only on lines that mention BIC or SWIFT.
fn scan_bic(ctx: &LineCtx<'_>, out: &mut Vec<Raw>) {
    let s = ctx.text;
    if !contains_any(s, &["BIC", "Bic", "SWIFT", "Swift"]) {
        return;
    }
    for (start, end) in word_tokens(s) {
        let token = &s[start..end];
        let b = token.as_bytes();
        let is_bic = matches!(b.len(), 8 | 11)
            && b[..6].iter().all(u8::is_ascii_uppercase)
            && b[6..]
                .iter()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            && !token.contains("SWIFT")
            && !token.contains("BIC");
        if is_bic {
            out.push(ctx.raw(
                CandidateKind::Bic,
                start,
                end,
                CandidateValue::Identifier(token.to_string()),
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// German tax ID (steuerliche Identifikationsnummer)
// ---------------------------------------------------------------------------

/// Groups of digit runs joined by single spaces, e.g. `12 345 678 901`.
fn digit_groups(s: &str) -> Vec<Vec<(usize, usize)>> {
    let b = s.as_bytes();
    let mut groups = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if !b[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let valid_start = number_start(s, i);
        let mut runs = Vec::new();
        loop {
            let run_start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            runs.push((run_start, i));
            if i + 1 < b.len() && b[i] == b' ' && b[i + 1].is_ascii_digit() {
                i += 1;
            } else {
                break;
            }
        }
        if valid_start {
            groups.push(runs);
        }
    }
    groups
}

/// Contiguous run subsets of a digit group holding exactly `digits` digits and
/// ending on a number boundary.
fn digit_spans(s: &str, group: &[(usize, usize)], digits: usize) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    for k in 0..group.len() {
        let mut total = 0;
        for (m, run) in group.iter().enumerate().skip(k) {
            total += run.1 - run.0;
            if total == digits {
                if m + 1 < group.len() || number_end(s, run.1) {
                    spans.push((group[k].0, run.1));
                }
                break;
            }
            if total > digits {
                break;
            }
        }
    }
    spans
}

fn compact_digits(s: &str) -> Vec<u8> {
    s.bytes()
        .filter(u8::is_ascii_digit)
        .map(|c| c - b'0')
        .collect()
}

/// Official Steuer-ID rules: 11 digits, no leading zero, exactly one digit
/// repeated (twice, or three times but not three in a row) in the first ten,
/// and an ISO/IEC 7064 MOD 11,10 check digit.
fn tax_id_valid(digits: &[u8]) -> bool {
    if digits.len() != 11 || digits[0] == 0 {
        return false;
    }
    let mut counts = [0u8; 10];
    for &d in &digits[..10] {
        counts[usize::from(d)] += 1;
    }
    let repeated: Vec<usize> = (0..10).filter(|&d| counts[d] > 1).collect();
    let [digit] = repeated[..] else {
        return false;
    };
    if counts[digit] > 3 {
        return false;
    }
    if counts[digit] == 3
        && digits[..10]
            .windows(3)
            .any(|w| w.iter().all(|&d| usize::from(d) == digit))
    {
        return false;
    }
    mod11_10_check(&digits[..10]) == digits[10]
}

/// ISO/IEC 7064 MOD 11,10 check digit.
fn mod11_10_check(digits: &[u8]) -> u8 {
    let mut product = 10u8;
    for &d in digits {
        let mut sum = (d + product) % 10;
        if sum == 0 {
            sum = 10;
        }
        product = (sum * 2) % 11;
    }
    let check = 11 - product;
    if check == 10 { 0 } else { check }
}

const TAX_ID_KEYWORDS: &[&str] = &["steuer", "identifikationsnummer", "idnr", "id-nr", "tax"];

fn scan_tax_id(ctx: &LineCtx<'_>, out: &mut Vec<Raw>) {
    let s = ctx.text;
    let context = ctx.mentions(TAX_ID_KEYWORDS);
    for group in digit_groups(s) {
        for (start, end) in digit_spans(s, &group, 11) {
            let digits = compact_digits(&s[start..end]);
            let value: String = digits.iter().map(|d| char::from(b'0' + d)).collect();
            if tax_id_valid(&digits) {
                out.push(Raw {
                    checksum: true,
                    ..ctx.raw(
                        CandidateKind::TaxId,
                        start,
                        end,
                        CandidateValue::Identifier(value),
                    )
                });
            } else if context && ctx.near_keyword(start, TAX_ID_KEYWORDS) {
                out.push(ctx.raw(
                    CandidateKind::Identifier,
                    start,
                    end,
                    CandidateValue::Identifier(value),
                ));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// German pension insurance number (Rentenversicherungs-/Sozialversicherungsnummer)
// ---------------------------------------------------------------------------

const SOCIAL_KEYWORDS: &[&str] = &[
    "versicherungsnummer",
    "rentenversicherung",
    "sozialversicherung",
    "rv-nr",
    "sv-nr",
    "rvnr",
    "svnr",
    "vsnr",
];

/// `NN DDMMYY L SS C`: area, birth date, surname initial, serial, check digit.
fn scan_social_insurance(ctx: &LineCtx<'_>, out: &mut Vec<Raw>) {
    let s = ctx.text;
    let b = s.as_bytes();
    let context = ctx.mentions(SOCIAL_KEYWORDS);
    for i in 0..b.len() {
        if !b[i].is_ascii_uppercase() || i == 0 {
            continue;
        }
        let glued = |c: Option<&u8>| c.is_some_and(|c| c.is_ascii_digit() || *c == b' ');
        if !glued(b.get(i - 1)) || !glued(b.get(i + 1)) {
            continue;
        }
        let Some((start, before)) = digits_before(b, i, 8) else {
            continue;
        };
        let Some((end, after)) = digits_after(b, i + 1, 3) else {
            continue;
        };
        if !number_start(s, start) || !number_end(s, end) {
            continue;
        }
        let day = before[2] * 10 + before[3];
        let month = before[4] * 10 + before[5];
        if !(1..=12).contains(&month) || !(1..=31).contains(&(day % 50)) || day > 81 {
            continue;
        }
        let letter = b[i];
        let valid = social_check(&before, letter, &after[..2]) == after[2];
        if !valid && !(context && ctx.near_keyword(start, SOCIAL_KEYWORDS)) {
            continue;
        }
        let mut value: String = before.iter().map(|d| char::from(b'0' + d)).collect();
        value.push(char::from(letter));
        value.extend(after.iter().map(|d| char::from(b'0' + d)));
        out.push(Raw {
            checksum: valid,
            ..ctx.raw(
                CandidateKind::SocialInsuranceNumber,
                start,
                end,
                CandidateValue::Identifier(value),
            )
        });
    }
}

/// Collects `count` digits ending right before `i` (one optional space before the
/// letter, single spaces between digits). Returns the start offset and digits.
fn digits_before(b: &[u8], i: usize, count: usize) -> Option<(usize, Vec<u8>)> {
    let mut j = if i > 0 && b[i - 1] == b' ' { i - 1 } else { i };
    let mut digits = Vec::with_capacity(count);
    while digits.len() < count && j > 0 {
        if b[j - 1].is_ascii_digit() {
            digits.push(b[j - 1] - b'0');
            j -= 1;
        } else if b[j - 1] == b' ' && j >= 2 && b[j - 2].is_ascii_digit() && !digits.is_empty() {
            j -= 1;
        } else {
            break;
        }
    }
    if digits.len() != count || (j > 0 && b[j - 1].is_ascii_digit()) {
        return None;
    }
    if j >= 2 && b[j - 1] == b' ' && b[j - 2].is_ascii_digit() {
        return None;
    }
    digits.reverse();
    Some((j, digits))
}

/// Mirror of [`digits_before`] starting at `i`.
fn digits_after(b: &[u8], i: usize, count: usize) -> Option<(usize, Vec<u8>)> {
    let mut j = if b.get(i) == Some(&b' ') { i + 1 } else { i };
    let mut digits = Vec::with_capacity(count);
    while digits.len() < count && j < b.len() {
        if b[j].is_ascii_digit() {
            digits.push(b[j] - b'0');
            j += 1;
        } else if b[j] == b' ' && b.get(j + 1).is_some_and(u8::is_ascii_digit) && !digits.is_empty()
        {
            j += 1;
        } else {
            break;
        }
    }
    if digits.len() != count || b.get(j).is_some_and(u8::is_ascii_digit) {
        return None;
    }
    if b.get(j) == Some(&b' ') && b.get(j + 1).is_some_and(u8::is_ascii_digit) {
        return None;
    }
    Some((j, digits))
}

/// Check digit: the letter becomes its two-digit alphabet position; weights
/// 2,1,2,5,7,1,2,1,2,1,2,1; sum of the products' digit sums modulo 10.
fn social_check(first8: &[u8], letter: u8, serial: &[u8]) -> u8 {
    const WEIGHTS: [u32; 12] = [2, 1, 2, 5, 7, 1, 2, 1, 2, 1, 2, 1];
    let position = letter - b'A' + 1;
    let digits = first8
        .iter()
        .copied()
        .chain([position / 10, position % 10])
        .chain(serial.iter().copied());
    let sum: u32 = digits
        .zip(WEIGHTS)
        .map(|(d, w)| {
            let product = u32::from(d) * w;
            product / 10 + product % 10
        })
        .sum();
    (sum % 10) as u8
}

// ---------------------------------------------------------------------------
// Dates (complete calendar dates only; no century is ever invented)
// ---------------------------------------------------------------------------

fn month_number(word: &str) -> Option<u32> {
    Some(match word.to_lowercase().as_str() {
        "januar" | "jänner" | "jaenner" | "january" | "jan" | "jän" => 1,
        "februar" | "feber" | "february" | "feb" | "febr" => 2,
        "märz" | "maerz" | "marz" | "march" | "mär" | "mrz" | "mar" => 3,
        "april" | "apr" => 4,
        "mai" | "may" => 5,
        "juni" | "june" | "jun" => 6,
        "juli" | "july" | "jul" => 7,
        "august" | "aug" => 8,
        "september" | "sept" | "sep" => 9,
        "oktober" | "october" | "okt" | "oct" => 10,
        "november" | "nov" => 11,
        "dezember" | "december" | "dez" | "dec" => 12,
        _ => return None,
    })
}

/// Reads exactly `min..=max` ASCII digits at `i` (the run must not continue).
fn read_digits(b: &[u8], i: usize, min: usize, max: usize) -> Option<(u32, usize)> {
    let mut j = i;
    let mut value = 0u32;
    while j < b.len() && b[j].is_ascii_digit() {
        if j - i >= max {
            return None;
        }
        value = value * 10 + u32::from(b[j] - b'0');
        j += 1;
    }
    (j - i >= min).then_some((value, j))
}

/// Reads a month name (letters, optional trailing `.`) at `i`.
fn read_month(s: &str, i: usize) -> Option<(u32, usize)> {
    let rest = s.get(i..)?;
    let len: usize = rest
        .chars()
        .take_while(|c| c.is_alphabetic())
        .take(12)
        .map(char::len_utf8)
        .sum();
    if len == 0 || !word_end(s, i + len) {
        return None;
    }
    let dot = usize::from(rest.as_bytes().get(len) == Some(&b'.'));
    month_number(&rest[..len]).map(|m| (m, i + len + dot))
}

fn skip_spaces(b: &[u8], mut i: usize, max: usize) -> usize {
    let start = i;
    while i < b.len() && b[i] == b' ' && i - start < max {
        i += 1;
    }
    i
}

/// Four-digit year at `i` followed by a number boundary.
fn read_year(s: &str, i: usize) -> Option<(i32, usize)> {
    let (year, end) = read_digits(s.as_bytes(), i, 4, 4)?;
    (number_end(s, end) && (1800..=2200).contains(&year)).then_some((year as i32, end))
}

/// Date starting with a number at `i`.
fn parse_numeric_date(s: &str, i: usize) -> Option<(usize, String)> {
    let b = s.as_bytes();
    let (first, j) = read_digits(b, i, 1, 4)?;
    if j - i == 4 {
        // 1988-03-14
        if b.get(j) != Some(&b'-') {
            return None;
        }
        let (month, k) = read_digits(b, j + 1, 1, 2)?;
        if b.get(k) != Some(&b'-') {
            return None;
        }
        let (day, end) = read_digits(b, k + 1, 1, 2)?;
        if !number_end(s, end) {
            return None;
        }
        return Some((end, iso_date(first as i32, month, day)?));
    }
    if j - i > 2 {
        return None;
    }
    let day = first;
    match b.get(j) {
        Some(b'.') => {
            let k = skip_spaces(b, j + 1, 1);
            if b.get(k).is_some_and(u8::is_ascii_digit) {
                // 14.03.1988 / 14.3.1988 (14.03.88 is rejected: no century invention)
                let (month, m) = read_digits(b, k, 1, 2)?;
                if b.get(m) != Some(&b'.') {
                    return None;
                }
                let y = skip_spaces(b, m + 1, 1);
                let (year, end) = read_year(s, y)?;
                Some((end, iso_date(year, month, day)?))
            } else {
                // 14. März 1988
                let (month, m) = read_month(s, k)?;
                let y = skip_spaces(b, m, 2);
                let (year, end) = read_year(s, y)?;
                (y > m).then_some(())?;
                Some((end, iso_date(year, month, day)?))
            }
        }
        Some(b'/') => {
            // 14/03/1988 (day first, as on German documents)
            let (month, m) = read_digits(b, j + 1, 1, 2)?;
            if b.get(m) != Some(&b'/') {
                return None;
            }
            let (year, end) = read_year(s, m + 1)?;
            Some((end, iso_date(year, month, day)?))
        }
        Some(b' ') => {
            // 14 March 1988
            let (month, m) = read_month(s, j + 1)?;
            let y = skip_spaces(b, m, 2);
            if y == m {
                return None;
            }
            let (year, end) = read_year(s, y)?;
            Some((end, iso_date(year, month, day)?))
        }
        _ => None,
    }
}

/// `March 14, 1988`.
fn parse_month_first_date(s: &str, i: usize) -> Option<(usize, String)> {
    let b = s.as_bytes();
    let (month, m) = read_month(s, i)?;
    let d = skip_spaces(b, m, 1);
    if d == m {
        return None;
    }
    let (day, k) = read_digits(b, d, 1, 2)?;
    let k = if b.get(k) == Some(&b',') { k + 1 } else { k };
    let y = skip_spaces(b, k, 2);
    if y == k {
        return None;
    }
    let (year, end) = read_year(s, y)?;
    Some((end, iso_date(year, month, day)?))
}

fn scan_dates(ctx: &LineCtx<'_>, out: &mut Vec<Raw>) {
    let s = ctx.text;
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let parsed = if c.is_ascii_digit() && number_start(s, i) {
            parse_numeric_date(s, i)
        } else if c.is_ascii_alphabetic() && word_start(s, i) {
            parse_month_first_date(s, i)
        } else {
            None
        };
        if let Some((end, date)) = parsed {
            out.push(ctx.raw(CandidateKind::Date, i, end, CandidateValue::Date(date)));
            i = end;
        } else if c.is_ascii_alphanumeric() {
            while i < b.len() && b[i].is_ascii_alphanumeric() {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
}

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
    YEAR_KEYWORDS.iter().any(|k| before.ends_with(k)).then(|| {
        (
            end,
            (format!("{year:04}-01-01"), format!("{year:04}-12-31")),
        )
    })
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

// ---------------------------------------------------------------------------
// Money (explicit currency marker required)
// ---------------------------------------------------------------------------

const CURRENCY_CODES: &[&str] = &[
    "EUR", "USD", "GBP", "CHF", "JPY", "CAD", "AUD", "NZD", "SEK", "NOK", "DKK", "ISK", "PLN",
    "CZK", "HUF", "RON", "BGN", "TRY", "CNY", "HKD", "SGD", "INR", "ZAR", "BRL", "MXN", "ILS",
    "AED", "KRW", "THB",
];

fn currency_symbol(c: char) -> Option<&'static str> {
    match c {
        '€' => Some("EUR"),
        '$' => Some("USD"),
        '£' => Some("GBP"),
        _ => None,
    }
}

fn is_gap(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\u{a0}' | '\u{202f}' | '\u{2009}')
}

/// Currency marker starting at `i`: returns (code, end).
fn currency_at(s: &str, i: usize) -> Option<(&'static str, usize)> {
    let c = next_char(s, i)?;
    if let Some(code) = currency_symbol(c) {
        return Some((code, i + c.len_utf8()));
    }
    if !word_start(s, i) {
        return None;
    }
    let rest = &s[i..];
    if let Some(code) = CURRENCY_CODES
        .iter()
        .find(|code| rest.starts_with(**code) && word_end(s, i + 3))
    {
        return Some((code, i + 3));
    }
    let euro = rest.get(..4)?;
    (euro.eq_ignore_ascii_case("euro") && word_end(s, i + 4)).then_some(("EUR", i + 4))
}

/// Currency marker ending at `i`: returns (code, start).
fn currency_before(s: &str, i: usize) -> Option<(&'static str, usize)> {
    let c = prev_char(s, i)?;
    if let Some(code) = currency_symbol(c) {
        return Some((code, i - c.len_utf8()));
    }
    for len in [3, 4] {
        let start = i.checked_sub(len)?;
        if s.is_char_boundary(start)
            && let Some((code, end)) = currency_at(s, start)
            && end == i
        {
            return Some((code, start));
        }
    }
    None
}

/// Skips up to three gap characters forward.
fn skip_gap_forward(s: &str, i: usize) -> usize {
    let mut j = i;
    for c in s[i..].chars().take(3) {
        if !is_gap(c) {
            break;
        }
        j += c.len_utf8();
    }
    j
}

fn skip_gap_backward(s: &str, i: usize) -> usize {
    let mut j = i;
    for c in s[..i].chars().rev().take(3) {
        if !is_gap(c) {
            break;
        }
        j -= c.len_utf8();
    }
    j
}

/// Parses an amount at `i` (optional sign). Returns (end, normalized decimal).
fn parse_amount(s: &str, i: usize) -> Option<(usize, String)> {
    let b = s.as_bytes();
    let mut j = i;
    let mut negative = false;
    if let Some(c) = next_char(s, j)
        && matches!(c, '-' | '+' | '−')
    {
        negative = c != '+';
        j += c.len_utf8();
    }
    if !b.get(j).is_some_and(u8::is_ascii_digit) {
        return None;
    }
    let mut groups: Vec<&str> = Vec::new();
    let mut seps: Vec<char> = Vec::new();
    loop {
        let start = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        groups.push(&s[start..j]);
        let Some(sep) = next_char(s, j).filter(|c| matches!(c, '.' | ',' | '\'' | '’')) else {
            break;
        };
        if !b.get(j + sep.len_utf8()).is_some_and(u8::is_ascii_digit) {
            break;
        }
        seps.push(sep);
        j += sep.len_utf8();
    }
    // German whole-euro notation: "12,-" / "12,–".
    let mut whole = false;
    if b.get(j) == Some(&b',')
        && let Some(dash) = next_char(s, j + 1).filter(|c| matches!(c, '-' | '–'))
    {
        whole = true;
        j += 1 + dash.len_utf8();
        if b.get(j) == Some(&b'-') {
            j += 1;
        }
    }
    let last_group = groups[groups.len() - 1];
    let (integer_groups, integer_seps, decimals) = match seps.last() {
        Some(&sep) if !whole && matches!(sep, '.' | ',') && last_group.len() <= 2 => {
            let n = groups.len();
            (
                &groups[..n - 1],
                &seps[..seps.len() - 1],
                Some((sep, last_group)),
            )
        }
        _ => (&groups[..], &seps[..], None),
    };
    if let Some(&first_sep) = integer_seps.first() {
        let uniform = integer_seps.iter().all(|&c| c == first_sep);
        let distinct = decimals.is_none_or(|(d, _)| d != first_sep);
        let shaped =
            integer_groups[0].len() <= 3 && integer_groups[1..].iter().all(|g| g.len() == 3);
        if !uniform || !distinct || !shaped {
            return None;
        }
    }
    let digits: String = integer_groups.concat();
    let trimmed = digits.trim_start_matches('0');
    let mut amount = String::new();
    if negative {
        amount.push('-');
    }
    amount.push_str(if trimmed.is_empty() { "0" } else { trimmed });
    if let Some((_, fraction)) = decimals {
        amount.push('.');
        amount.push_str(fraction);
    }
    Some((j, amount))
}

fn scan_money(ctx: &LineCtx<'_>, out: &mut Vec<Raw>) {
    let s = ctx.text;
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let signed = matches!(b[i], b'-' | b'+') || s.get(i..).is_some_and(|r| r.starts_with('−'));
        let starts = if signed {
            let sign_len = if b[i] == b'-' || b[i] == b'+' { 1 } else { 3 };
            b.get(i + sign_len).is_some_and(u8::is_ascii_digit)
                && prev_char(s, i).is_none_or(|c| !c.is_alphanumeric())
        } else {
            b[i].is_ascii_digit() && number_start(s, i)
        };
        if !starts {
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
        let marker = match currency_at(s, after) {
            Some((code, marker_end)) => Some((code, i, marker_end)),
            None if number_end(s, end) => {
                let before = skip_gap_backward(s, i);
                currency_before(s, before).map(|(code, marker_start)| (code, marker_start, end))
            }
            None => None,
        };
        if let Some((currency, start, stop)) = marker {
            out.push(ctx.raw(
                CandidateKind::Money,
                start,
                stop,
                CandidateValue::Money {
                    amount,
                    currency: currency.to_string(),
                },
            ));
        }
        i = end.max(i + 1);
    }
}

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
/// Used to resolve a document's currency (document marker, else type default).
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
        return euro_cent_pair(t, end, &amount)
            .filter(|(stop, _)| *stop == t.len())
            .map(|(_, a)| a);
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
            out.push(ctx.raw(
                CandidateKind::Amount,
                i,
                end,
                CandidateValue::Amount(amount),
            ));
        } else if !marked
            && ctx.doc.euro_cent
            && let Some((stop, joined)) = euro_cent_pair(s, end, &amount)
        {
            out.push(ctx.raw(
                CandidateKind::Amount,
                i,
                stop,
                CandidateValue::Amount(joined),
            ));
            i = stop;
            continue;
        }
        i = end.max(i + 1);
    }
}

// ---------------------------------------------------------------------------
// Labeled identifiers (contract, customer, file and reference numbers)
// ---------------------------------------------------------------------------

const IDENTIFIER_KEYWORDS: &[&str] = &[
    "nummer",
    "nr.",
    "nr:",
    "number",
    "no.",
    "vertrag",
    "police",
    "versicherungsschein",
    "kunden",
    "aktenzeichen",
    "az.",
    "referenz",
    "reference",
    "mitglied",
    "personalnummer",
    "steuernummer",
    "kennziffer",
];

/// Whether a whitespace-delimited, ASCII-lowercased token reads as an identifier label.
fn is_identifier_label(token: &str) -> bool {
    let keyword = IDENTIFIER_KEYWORDS.iter().any(|keyword| {
        find_bytes(token.as_bytes(), keyword.as_bytes(), 0).is_some_and(|p| {
            !matches!(*keyword, "no." | "az.")
                || p == 0
                || !token.as_bytes()[p - 1].is_ascii_alphabetic()
        })
    });
    keyword || token.trim_end_matches([':', '.']).ends_with("nr")
}

fn is_identifier_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'-' | b'/' | b'.')
}

/// Token group at `from`: tokens of `[A-Za-z0-9-/.]` joined by single spaces;
/// each token has a digit or is a short uppercase code such as `KV`. At most 40
/// bytes long.
fn identifier_group(s: &str, from: usize) -> Option<(usize, usize)> {
    let b = s.as_bytes();
    let mut start = from;
    while start < b.len() && matches!(b[start], b'.' | b'-' | b'/') {
        start += 1;
    }
    let mut i = start;
    let mut end = start;
    let mut has_digit = false;
    loop {
        let token_start = i;
        while i < b.len() && is_identifier_char(b[i]) && i - start <= 40 {
            i += 1;
        }
        if i - start > 40 {
            return None;
        }
        if i == token_start || !word_end(s, i) {
            break;
        }
        let token = &b[token_start..i];
        let digits = token.iter().any(u8::is_ascii_digit);
        let code = token.len() <= 4 && token.iter().all(u8::is_ascii_uppercase);
        if !digits && !code {
            break;
        }
        // Codes such as `KV` may lead or sit inside the group, but it ends on a digit token.
        if digits {
            has_digit = true;
            end = i;
        }
        if b.get(i) == Some(&b' ') && b.get(i + 1).is_some_and(|&c| is_identifier_char(c)) {
            i += 1;
        } else {
            break;
        }
    }
    while end > start && matches!(b[end - 1], b'.' | b'-' | b'/') {
        end -= 1;
    }
    (has_digit && (4..=40).contains(&(end - start))).then_some((start, end))
}

fn compact_identifier(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// Whitespace-delimited tokens as byte spans.
fn space_tokens(s: &str) -> Vec<(usize, usize)> {
    let b = s.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i > start {
            tokens.push((start, i));
        }
    }
    tokens
}

fn scan_identifiers(ctx: &LineCtx<'_>, out: &mut Vec<Raw>) {
    let s = ctx.text;
    let b = s.as_bytes();
    let mut emitted_until = 0;
    for (ts, te) in space_tokens(s) {
        if ts < emitted_until || !is_identifier_label(&ctx.lower[ts..te]) {
            continue;
        }
        let label_end = s[ts..te].find(':').map_or(te, |c| ts + c + 1);
        let limit = label_end + 60;
        let mut i = label_end;
        while i < b.len() && i < limit {
            while i < b.len()
                && (b[i].is_ascii_whitespace() || matches!(b[i], b':' | b'#' | b'=' | b'.'))
            {
                i += 1;
            }
            if let Some((start, end)) = identifier_group(s, i) {
                let label = trim_span(s, (ts, s[ts..label_end].trim_end_matches(':').len() + ts));
                emit_identifier(ctx, (start, end), Some(ctx.abs(label)), out);
                emitted_until = end;
                break;
            }
            while i < b.len() && i < limit && !b[i].is_ascii_whitespace() {
                i += 1;
            }
        }
    }
    // Value on the line after a short label line such as "Versicherungsnummer".
    let prev = ctx.prev_line();
    if prev.len() <= PREV_LABEL_MAX
        && !prev.bytes().any(|c| c.is_ascii_digit())
        && space_tokens(prev)
            .iter()
            .any(|&(a, e)| is_identifier_label(&prev[a..e].to_ascii_lowercase()))
    {
        let first = s.len() - s.trim_start().len();
        if let Some(span) = identifier_group(s, first) {
            emit_identifier(ctx, span, None, out);
        }
    }
}

fn emit_identifier(
    ctx: &LineCtx<'_>,
    (start, end): (usize, usize),
    fallback_label: Option<(usize, usize)>,
    out: &mut Vec<Raw>,
) {
    let label = ctx.doc.label_for(ctx.base + start).or(fallback_label);
    out.push(Raw {
        label,
        ..ctx.raw(
            CandidateKind::Identifier,
            start,
            end,
            CandidateValue::Identifier(compact_identifier(&ctx.text[start..end])),
        )
    });
}

// ---------------------------------------------------------------------------
// Person names
// ---------------------------------------------------------------------------

/// Case- and diacritics-insensitive folding (`ä` → `ae`, `ß` → `ss`, …).
fn fold_char(c: char, out: &mut String) {
    match c {
        'ä' | 'Ä' => out.push_str("ae"),
        'ö' | 'Ö' => out.push_str("oe"),
        'ü' | 'Ü' => out.push_str("ue"),
        'ß' | 'ẞ' => out.push_str("ss"),
        'à' | 'á' | 'â' | 'ã' | 'å' | 'À' | 'Á' | 'Â' | 'Ã' | 'Å' => out.push('a'),
        'ç' | 'Ç' | 'č' | 'Č' | 'ć' | 'Ć' => out.push('c'),
        'è' | 'é' | 'ê' | 'ë' | 'È' | 'É' | 'Ê' | 'Ë' => out.push('e'),
        'ì' | 'í' | 'î' | 'ï' | 'Ì' | 'Í' | 'Î' | 'Ï' => out.push('i'),
        'ñ' | 'Ñ' => out.push('n'),
        'ò' | 'ó' | 'ô' | 'õ' | 'ø' | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ø' => out.push('o'),
        'ù' | 'ú' | 'û' | 'Ù' | 'Ú' | 'Û' => out.push('u'),
        'š' | 'Š' | 'ś' | 'Ś' => out.push('s'),
        'ž' | 'Ž' | 'ź' | 'Ź' | 'ż' | 'Ż' => out.push('z'),
        'ł' | 'Ł' => out.push('l'),
        c => out.extend(c.to_lowercase()),
    }
}

/// Folds `s` for name matching: whitespace and MRZ fillers (`<`) collapse to one
/// space, commas are normalized to `, `. `map[k]` is the source offset of folded
/// byte `k`.
fn fold_with_map(s: &str) -> (String, Vec<u32>) {
    let mut folded = String::with_capacity(s.len());
    let mut map = Vec::with_capacity(s.len());
    let mut pending_space: Option<usize> = None;
    for (i, c) in s.char_indices() {
        if c.is_whitespace() || c == '<' {
            pending_space.get_or_insert(i);
            continue;
        }
        if c == ',' {
            folded.push(',');
            map.push(i as u32);
            pending_space = Some(i + 1);
            continue;
        }
        if let Some(space) = pending_space.take()
            && !folded.is_empty()
        {
            folded.push(' ');
            map.push(space as u32);
        }
        let before = folded.len();
        fold_char(c, &mut folded);
        map.extend(std::iter::repeat_n(i as u32, folded.len() - before));
    }
    (folded, map)
}

fn is_folded_word_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c >= 0x80
}

/// Matches the user's and household members' known names in all orders.
struct NameMatcher {
    names: Vec<String>,
    /// (folded pattern, index into `names`)
    patterns: Vec<(String, usize)>,
}

impl NameMatcher {
    fn new(known: &[&str]) -> Self {
        let mut names = Vec::new();
        let mut patterns: Vec<(String, usize)> = Vec::new();
        for name in known {
            let name = name.trim();
            let (folded, _) = fold_with_map(name);
            let words: Vec<&str> = folded.split([' ', ',']).filter(|w| !w.is_empty()).collect();
            if words.iter().map(|w| w.len()).sum::<usize>() < 3 {
                continue;
            }
            let index = names.len();
            names.push(name.to_string());
            let mut variants = vec![words.join(" ")];
            if words.len() >= 2 {
                let (surname, given) = if folded.contains(',') {
                    (words[0], &words[1..])
                } else {
                    (words[words.len() - 1], &words[..words.len() - 1])
                };
                let all = given.join(" ");
                variants.push(format!("{all} {surname}"));
                variants.push(format!("{surname}, {all}"));
                variants.push(format!("{surname} {all}"));
                if given.len() > 1 {
                    variants.push(format!("{} {surname}", given[0]));
                    variants.push(format!("{surname}, {}", given[0]));
                    variants.push(format!("{surname} {}", given[0]));
                }
            }
            for variant in variants {
                if !patterns.iter().any(|(p, _)| *p == variant) {
                    patterns.push((variant, index));
                }
            }
        }
        Self { names, patterns }
    }

    fn scan(&self, ctx: &LineCtx<'_>, out: &mut Vec<Raw>) {
        let s = ctx.text;
        if self.patterns.is_empty() || s.len() >= u32::MAX as usize {
            return;
        }
        let (folded, map) = fold_with_map(s);
        let bytes = folded.as_bytes();
        for (pattern, index) in &self.patterns {
            for fs in occurrences(&folded, pattern) {
                let fe = fs + pattern.len();
                let bounded = (fs == 0 || !is_folded_word_byte(bytes[fs - 1]))
                    && (fe == bytes.len() || !is_folded_word_byte(bytes[fe]));
                if !bounded {
                    continue;
                }
                let start = map[fs] as usize;
                let last = map[fe - 1] as usize;
                let end = last + next_char(s, last).map_or(1, char::len_utf8);
                out.push(ctx.raw(
                    CandidateKind::PersonName,
                    start,
                    end,
                    CandidateValue::Text(self.names[*index].clone()),
                ));
            }
        }
    }
}

const SALUTATIONS: &[&str] = &[
    "Herr", "Herrn", "Frau", "HERR", "HERRN", "FRAU", "Mr", "Mrs", "Ms", "MR", "MRS", "MS",
];

/// Academic titles skipped between a salutation and the name.
const TITLES: &[&str] = &[
    "dr.",
    "prof.",
    "dipl.",
    "dipl.-ing.",
    "dr.-ing.",
    "ing.",
    "mag.",
    "med.",
    "rer.",
    "nat.",
    "jur.",
    "phil.",
];

/// Lowercase labels after which a name is printed; `name` additionally needs `:`.
const NAME_LABELS: &[&str] = &[
    "versicherte person",
    "versicherungsnehmer",
    "kontoinhaber",
    "arbeitnehmer",
    "nachname",
    "vorname",
    "inhaber",
    "halter",
    "name",
];

const NAME_STOP_WORDS: &[&str] = &[
    "straße",
    "strasse",
    "str",
    "geb",
    "geboren",
    "geburtsdatum",
    "geburtsname",
    "geburtsort",
    "anschrift",
    "adresse",
    "vorname",
    "nachname",
    "name",
    "datum",
    "und",
    "tel",
    "telefon",
    "iban",
    "bic",
    "nr",
    "plz",
    "ort",
    "wohnort",
    "vertrag",
    "vertragsnummer",
    "kundennummer",
    "sehr",
    "geehrte",
    "geehrter",
    "dr",
    "prof",
    "herr",
    "herrn",
    "frau",
    "mr",
    "mrs",
    "ms",
];

// ---------------------------------------------------------------------------
// Organizations and German addresses.

/// Legal forms and institution words that mark an organization name.
const ORGANIZATION_WORDS: &[&str] = &[
    "gmbh",
    "mbh",
    "ag",
    "se",
    "kg",
    "ohg",
    "gbr",
    "ev",
    "eg",
    "kgaa",
    "ltd",
    "inc",
    "llc",
    "versicherung",
    "versicherungen",
    "krankenkasse",
    "ersatzkasse",
    "bkk",
    "aok",
    "ikk",
    "bank",
    "sparkasse",
    "volksbank",
    "raiffeisenbank",
    "finanzamt",
    "stadtwerke",
    "hausverwaltung",
    "wohnungsbaugesellschaft",
    "landratsamt",
    "bürgeramt",
    "buergeramt",
    "stadtverwaltung",
    "rentenversicherung",
    "agentur",
    "jobcenter",
    "familienkasse",
];
const ORGANIZATION_LABELS: &[&str] = &[
    "arbeitgeber",
    "versicherer",
    "absender",
    "vermieter",
    "anbieter",
    "bank",
    "kreditinstitut",
    "krankenkasse",
    "employer",
    "insurer",
];
const ORGANIZATION_MAX_BYTES: usize = 80;
const ORGANIZATION_MAX_WORDS: usize = 8;

/// A line (or the value after an organization label) naming an organization.
fn scan_organizations(ctx: &LineCtx<'_>, out: &mut Vec<Raw>) {
    let s = ctx.text;
    let tokens = word_tokens(s);
    let lower = &ctx.lower;
    let Some(marker) = tokens.iter().position(|&(a, b)| {
        let word = lower[a..b].to_string();
        ORGANIZATION_WORDS
            .iter()
            .any(|w| word == *w || (w.len() > 4 && word.ends_with(w)))
    }) else {
        return;
    };
    // Start after an organization label ("Arbeitgeber: Acme GmbH"), else at the line start.
    let mut start = s.len() - s.trim_start().len();
    if let Some(colon) = s[..tokens[marker].0].rfind(':') {
        let label = lower[..colon].trim();
        if ORGANIZATION_LABELS.iter().any(|l| label.ends_with(l)) || colon < 40 {
            start = colon + 1;
        }
    }
    start += s[start..].len() - s[start..].trim_start().len();
    // End after the legal form, or at the line end for institution words.
    let marker_end = tokens[marker].1;
    const LEGAL: &[&str] = &[
        "gmbh", "mbh", "ag", "se", "kg", "co", "ohg", "ev", "eg", "kgaa",
    ];
    let end = tokens[marker + 1..]
        .iter()
        .take_while(|&&(a, b)| LEGAL.contains(&&lower[a..b]))
        .last()
        .map_or(marker_end, |t| t.1);
    let end = if s.as_bytes().get(end) == Some(&b'.') {
        end + 1
    } else {
        end
    };
    let (start, end) = trim_span(s, (start, end));
    let name = &s[start..end];
    let words = word_tokens(name).len();
    if name.is_empty()
        || name.len() > ORGANIZATION_MAX_BYTES
        || words == 0
        || words > ORGANIZATION_MAX_WORDS
        || !name.chars().next().is_some_and(|c| c.is_alphabetic())
        || digit_count(name) * 2 > name.len()
    {
        return;
    }
    out.push(ctx.raw(
        CandidateKind::Organization,
        start,
        end,
        CandidateValue::Text(name.to_string()),
    ));
}

const STREET_SUFFIXES: &[&str] = &[
    "straße", "strasse", "str", "weg", "allee", "platz", "gasse", "ring", "damm", "ufer",
    "chaussee", "steig", "pfad", "markt", "berg", "hof", "park", "stieg", "twiete",
];

fn house_number(s: &str, start: usize) -> Option<usize> {
    let b = s.as_bytes();
    let mut i = start;
    let digits = |i: &mut usize| {
        let from = *i;
        while *i < b.len() && b[*i].is_ascii_digit() && *i - from < 4 {
            *i += 1;
        }
        *i > from
    };
    if !digits(&mut i) {
        return None;
    }
    if i < b.len()
        && b[i].is_ascii_alphabetic()
        && (i + 1 == b.len() || !b[i + 1].is_ascii_alphanumeric())
    {
        i += 1;
    }
    if i + 1 < b.len() && b[i] == b'-' && b[i + 1].is_ascii_digit() {
        i += 1;
        digits(&mut i);
    }
    (i == b.len() || !b[i].is_ascii_alphanumeric()).then_some(i)
}

/// `Musterstraße 12a`, `Am Hang-Weg 3` and `12345 Berlin` on German letters.
fn scan_address(ctx: &LineCtx<'_>, out: &mut Vec<Raw>) {
    let s = ctx.text;
    let lower = &ctx.lower;
    let tokens = word_tokens(s);
    for (index, &(a, b)) in tokens.iter().enumerate() {
        let word = &lower[a..b];
        // Street: a token ending in a street suffix, then a house number.
        if STREET_SUFFIXES.iter().any(|x| word.ends_with(x))
            && let Some(&(na, _)) = tokens.get(index + 1)
            && s[b..na]
                .trim_matches(|c: char| c == '.' || c.is_whitespace())
                .is_empty()
            && let Some(number_end) = house_number(s, na)
        {
            // Include up to two preceding capitalized words ("Karl Marx Allee").
            let mut start = a;
            for &(pa, pb) in tokens[..index].iter().rev().take(2) {
                let between = &s[pb..start];
                if between.chars().all(|c| c == ' ' || c == '-')
                    && s[pa..].starts_with(|c: char| c.is_uppercase())
                {
                    start = pa;
                } else {
                    break;
                }
            }
            if s[start..].starts_with(|c: char| c.is_uppercase()) {
                out.push(ctx.raw(
                    CandidateKind::Street,
                    start,
                    number_end,
                    CandidateValue::Text(s[start..number_end].to_string()),
                ));
            }
        }
        // Postal code and city: five digits at a line or comma boundary, then a name.
        let token = &s[a..b];
        let boundary = s[..a].trim_end().is_empty()
            || s[..a].trim_end().ends_with(',')
            || s[..a].trim_end().ends_with("D-")
            || s[..a].trim_end().ends_with("DE-");
        if token.len() == 5
            && token.bytes().all(|c| c.is_ascii_digit())
            && boundary
            && let Some(&(ca, _)) = tokens.get(index + 1)
            && s[b..ca].trim().is_empty()
            && s[ca..].starts_with(|c: char| c.is_uppercase())
        {
            out.push(ctx.raw(
                CandidateKind::PostalCode,
                a,
                b,
                CandidateValue::Identifier(token.to_string()),
            ));
            let city_end = s[ca..].find([',', '(', '/']).map_or(s.len(), |i| ca + i);
            let (cs, ce) = trim_span(s, (ca, city_end));
            let city = &s[cs..ce];
            if !city.is_empty() && word_tokens(city).len() <= 4 && digit_count(city) == 0 {
                out.push(ctx.raw(
                    CandidateKind::City,
                    cs,
                    ce,
                    CandidateValue::Text(city.to_string()),
                ));
            }
        }
    }
}

/// Names printed after salutations or name labels ("Herr Max Mustermann",
/// "Kontoinhaber: Erika Mustermann").
fn scan_name_triggers(ctx: &LineCtx<'_>, out: &mut Vec<Raw>) {
    let s = ctx.text;
    for (ts, te) in word_tokens(s) {
        let after = if SALUTATIONS.contains(&&s[ts..te]) {
            Some(if s.as_bytes().get(te) == Some(&b'.') {
                te + 1
            } else {
                te
            })
        } else {
            name_label_end(ctx, ts)
        };
        if let Some((start, end)) = after.and_then(|i| name_after(s, i)) {
            out.push(ctx.raw(
                CandidateKind::PersonName,
                start,
                end,
                CandidateValue::Text(s[start..end].to_string()),
            ));
        }
    }
}

/// End of a name label starting at `ts`, including a feminine/plural suffix.
fn name_label_end(ctx: &LineCtx<'_>, ts: usize) -> Option<usize> {
    let rest = &ctx.lower[ts..];
    let label = NAME_LABELS.iter().find(|l| rest.starts_with(**l))?;
    let mut end = ts + label.len();
    if let Some(suffix) = ["(in)", "/in", "in", "n"]
        .iter()
        .find(|x| ctx.lower[end..].starts_with(**x) && word_end(ctx.text, end + x.len()))
    {
        end += suffix.len();
    }
    if !word_end(ctx.text, end) {
        return None;
    }
    if *label == "name" {
        let colon = skip_spaces(ctx.text.as_bytes(), end, 3);
        return (ctx.text.as_bytes().get(colon) == Some(&b':')).then_some(colon + 1);
    }
    Some(end)
}

/// Reads a capitalized name word at `i` (letters, `-`, `'`).
fn name_word(s: &str, i: usize) -> Option<usize> {
    let mut chars = s.get(i..)?.char_indices();
    let (_, first) = chars.next()?;
    if !first.is_uppercase() {
        return None;
    }
    let mut end = i + first.len_utf8();
    for (k, c) in chars.take(63) {
        if c.is_alphabetic() || matches!(c, '-' | '\'' | '’') {
            end = i + k + c.len_utf8();
        } else {
            break;
        }
    }
    let letters = s[i..end].chars().filter(|c| c.is_alphabetic()).count();
    (letters >= 2 && word_end(s, end)).then_some(end)
}

/// Up to four capitalized words after a trigger, skipping separators and
/// academic titles such as `Dr.`.
fn name_after(s: &str, from: usize) -> Option<(usize, usize)> {
    let b = s.as_bytes();
    let mut i = from;
    while i < b.len() && i - from < 8 && matches!(b[i], b' ' | b'\t' | b':' | b'.') {
        i += 1;
    }
    for _ in 0..3 {
        let head = &b[i..(i + 12).min(b.len())];
        let token_end = match head.iter().position(|&c| c == b' ' || c == b'\t') {
            Some(k) => i + k,
            None if head.len() < 12 => b.len(),
            None => break,
        };
        let token = s.get(i..token_end).unwrap_or_default().to_lowercase();
        if !TITLES.contains(&token.as_str()) {
            break;
        }
        i = skip_spaces(b, token_end, 2);
    }
    let start = i;
    let mut end = None;
    let mut words = 0;
    while words < 4 {
        let Some(word_end_at) = name_word(s, i) else {
            break;
        };
        let word = s[i..word_end_at].to_lowercase();
        if b.get(word_end_at) == Some(&b':') || NAME_STOP_WORDS.contains(&word.as_str()) {
            break;
        }
        words += 1;
        end = Some(word_end_at);
        if words == 1 && s[word_end_at..].starts_with(", ") {
            i = word_end_at + 2;
        } else if b.get(word_end_at) == Some(&b' ') {
            i = word_end_at + 1;
        } else {
            break;
        }
    }
    end.map(|end| (start, end))
}

// ---------------------------------------------------------------------------
// Vehicle plates and VINs
// ---------------------------------------------------------------------------

fn is_plate_letter(c: char) -> bool {
    c.is_ascii_uppercase() || matches!(c, 'Ä' | 'Ö' | 'Ü')
}

/// German plate `B-MX 1988`: area (1-3 letters), `-` or space, 1-2 letters,
/// optional space, 1-4 digits, optional `E`/`H` suffix.
fn parse_plate(s: &str, i: usize, allow_space: bool) -> Option<(usize, String)> {
    let b = s.as_bytes();
    let mut j = i;
    let mut area = String::new();
    for c in s.get(i..)?.chars() {
        if !is_plate_letter(c) || area.chars().count() >= 3 {
            break;
        }
        area.push(c);
        j += c.len_utf8();
    }
    let sep = next_char(s, j)?;
    if area.is_empty() || is_plate_letter(sep) || !(sep == '-' || (allow_space && sep == ' ')) {
        return None;
    }
    j += 1;
    let middle_start = j;
    while j < b.len() && b[j].is_ascii_uppercase() && j - middle_start < 3 {
        j += 1;
    }
    let middle = &s[middle_start..j];
    if !(1..=2).contains(&middle.len()) {
        return None;
    }
    if b.get(j) == Some(&b' ') {
        j += 1;
    }
    let (_, digits_end) = read_digits(b, j, 1, 4)?;
    if b[j] == b'0' {
        return None;
    }
    let digits = &s[j..digits_end];
    let mut end = digits_end;
    let mut suffix = "";
    if matches!(b.get(end), Some(b'E' | b'H')) && word_end(s, end + 1) {
        suffix = &s[end..end + 1];
        end += 1;
    }
    word_end(s, end).then(|| (end, format!("{area}-{middle} {digits}{suffix}")))
}

fn scan_plates(ctx: &LineCtx<'_>, out: &mut Vec<Raw>) {
    let s = ctx.text;
    let allow_space = contains_any(&ctx.lower, &["kennzeichen", "amtl", "kfz"]);
    for (ts, _) in word_tokens(s) {
        if let Some((end, plate)) = parse_plate(s, ts, allow_space) {
            out.push(ctx.raw(
                CandidateKind::Plate,
                ts,
                end,
                CandidateValue::Identifier(plate),
            ));
        }
    }
}

fn has_word(s: &str, word: &str) -> bool {
    occurrences(s, word).any(|p| word_start(s, p) && word_end(s, p + word.len()))
}

/// 17-character VIN on lines that mention FIN/VIN or the vehicle identification number.
fn scan_vin(ctx: &LineCtx<'_>, out: &mut Vec<Raw>) {
    let s = ctx.text;
    let trigger = has_word(s, "FIN")
        || has_word(s, "VIN")
        || contains_any(&ctx.lower, &["fahrzeug-ident", "identifizierungsnummer"]);
    if !trigger {
        return;
    }
    for (ts, te) in word_tokens(s) {
        let token = &s[ts..te];
        if token.len() != 17 || !token.is_ascii() {
            continue;
        }
        let vin = token.to_ascii_uppercase();
        let allowed = vin.bytes().all(|c| {
            c.is_ascii_digit() || (c.is_ascii_uppercase() && !matches!(c, b'I' | b'O' | b'Q'))
        });
        let letters = vin.bytes().filter(u8::is_ascii_uppercase).count();
        if allowed && letters >= 1 && digit_count(&vin) >= 2 {
            out.push(ctx.raw(CandidateKind::Vin, ts, te, CandidateValue::Identifier(vin)));
        }
    }
}

// ---------------------------------------------------------------------------
// Email and phone
// ---------------------------------------------------------------------------

fn scan_email(ctx: &LineCtx<'_>, out: &mut Vec<Raw>) {
    let s = ctx.text;
    let b = s.as_bytes();
    let local_char =
        |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'%' | b'+' | b'-');
    let domain_char = |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-');
    for (at, _) in s.match_indices('@') {
        let mut start = at;
        while start > 0 && at - start < 64 && local_char(b[start - 1]) {
            start -= 1;
        }
        while start < at && b[start] == b'.' {
            start += 1;
        }
        let mut end = at + 1;
        while end < b.len() && end - at < 255 && domain_char(b[end]) {
            end += 1;
        }
        while end > at + 1 && matches!(b[end - 1], b'.' | b'-') {
            end -= 1;
        }
        let domain = &s[at + 1..end];
        let tld_ok = domain.rfind('.').is_some_and(|dot| {
            let tld = &domain[dot + 1..];
            tld.len() >= 2 && tld.bytes().all(|c| c.is_ascii_alphabetic())
        });
        let valid = start < at
            && b[at - 1] != b'.'
            && tld_ok
            && !domain.starts_with(['.', '-'])
            && !domain.contains("..")
            && word_start(s, start)
            && word_end(s, end);
        if valid {
            out.push(ctx.raw(
                CandidateKind::Email,
                start,
                end,
                CandidateValue::Text(s[start..end].to_string()),
            ));
        }
    }
}

const PHONE_KEYWORDS: &[&str] = &["tel", "fon", "phone", "mobil", "handy", "fax", "ruf"];

/// Candidate ends (after each digit run) of a phone-like run starting at `i`.
fn phone_run_ends(b: &[u8], i: usize) -> Vec<usize> {
    let mut ends = Vec::new();
    let mut j = i + usize::from(b[i] == b'+');
    while j < b.len() && j - i < 40 {
        let next = b.get(j + 1).copied();
        match b[j] {
            b'0'..=b'9' => {
                j += 1;
                if !b.get(j).is_some_and(u8::is_ascii_digit) {
                    ends.push(j);
                }
            }
            b' ' if b[j - 1] != b' '
                && next.is_some_and(|c| c.is_ascii_digit() || matches!(c, b'(' | b'/' | b'-')) =>
            {
                j += 1
            }
            b'/' | b'-' if next.is_some_and(|c| c.is_ascii_digit() || c == b' ') => j += 1,
            b'(' | b')' => j += 1,
            _ => break,
        }
    }
    ends
}

fn phone_compact(text: &str) -> String {
    let international = text.starts_with('+') || text.starts_with("00");
    let text = if international {
        text.replacen("(0)", "", 1)
    } else {
        text.to_string()
    };
    text.chars()
        .filter(|c| c.is_ascii_digit() || *c == '+')
        .collect()
}

/// German/international phone numbers (+49 …, 0049 …, 030 …) with 8-15 digits.
fn scan_phone(ctx: &LineCtx<'_>, out: &mut Vec<Raw>) {
    let s = ctx.text;
    let b = s.as_bytes();
    let keyword = contains_any(&ctx.lower, PHONE_KEYWORDS);
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let start_ok = match c {
            b'+' => b.get(i + 1).is_some_and(u8::is_ascii_digit) && word_start(s, i),
            b'(' => b.get(i + 1) == Some(&b'0') && word_start(s, i),
            b'0' => number_start(s, i),
            _ => false,
        };
        if !start_ok {
            i += 1;
            continue;
        }
        let ends = phone_run_ends(b, i);
        let found = ends.iter().rev().find_map(|&end| {
            let compact = phone_compact(&s[i..end]);
            let digits = digit_count(&compact);
            let prefix_ok = compact.starts_with('+')
                || compact.starts_with("00")
                || (compact.starts_with('0')
                    && (keyword || ["015", "016", "017"].iter().any(|p| compact.starts_with(p))));
            ((8..=15).contains(&digits) && prefix_ok && number_end(s, end))
                .then_some((end, compact))
        });
        match found {
            Some((end, compact)) => {
                out.push(ctx.raw(
                    CandidateKind::Phone,
                    i,
                    end,
                    CandidateValue::Identifier(compact),
                ));
                i = end;
            }
            None => i = ends.last().copied().unwrap_or(i + 1).max(i + 1),
        }
    }
}

// ---------------------------------------------------------------------------
// Label/value catch-all
// ---------------------------------------------------------------------------

/// Cells separated by a tab or two or more spaces, trimmed.
fn split_cells(s: &str) -> Vec<(usize, usize)> {
    let b = s.as_bytes();
    let mut cells = Vec::new();
    let mut start = 0;
    let mut i = 0;
    let push = |cells: &mut Vec<(usize, usize)>, a: usize, e: usize| {
        let span = trim_span(s, (a, e));
        if span.1 > span.0 {
            cells.push(span);
        }
    };
    while i < b.len() {
        let gap = b[i] == b'\t' || (b[i] == b' ' && b.get(i + 1) == Some(&b' '));
        if gap {
            push(&mut cells, start, i);
            while i < b.len() && matches!(b[i], b' ' | b'\t') {
                i += 1;
            }
            start = i;
        } else {
            i += 1;
        }
    }
    push(&mut cells, start, b.len());
    cells
}

/// First `:` in a cell that separates a label (not a time or URL).
fn label_colon(cell: &str) -> Option<usize> {
    let b = cell.as_bytes();
    let c = cell.find(':')?;
    let time = c > 0 && b[c - 1].is_ascii_digit();
    (!time && b.get(c + 1) != Some(&b'/')).then_some(c)
}

fn is_label_text(s: &str) -> bool {
    (2..=40).contains(&s.len()) && has_letter(s)
}

fn scan_label_values(ctx: &LineCtx<'_>, out: &mut Vec<Raw>) {
    let s = ctx.text;
    let cells = split_cells(s);
    let mut k = 0;
    while k < cells.len() {
        let (cs, ce) = cells[k];
        let cell = &s[cs..ce];
        if let Some(c) = label_colon(cell) {
            let label = trim_span(s, (cs, cs + c));
            let mut value = trim_span(s, (cs + c + 1, ce));
            if value.0 == value.1 && k + 1 < cells.len() {
                k += 1;
                value = cells[k];
            }
            emit_label_value(ctx, label, value, out);
            k += 1;
        } else if k + 1 < cells.len()
            && is_label_text(cell)
            && label_colon(&s[cells[k + 1].0..cells[k + 1].1]).is_none()
        {
            emit_label_value(ctx, (cs, ce), cells[k + 1], out);
            k += 2;
        } else {
            k += 1;
        }
    }
}

fn emit_label_value(
    ctx: &LineCtx<'_>,
    label: (usize, usize),
    (vs, ve): (usize, usize),
    out: &mut Vec<Raw>,
) {
    if !is_label_text(&ctx.text[label.0..label.1]) || !(1..=120).contains(&(ve - vs)) {
        return;
    }
    out.push(Raw {
        label: Some(ctx.abs(label)),
        ..ctx.raw(
            CandidateKind::LabelValue,
            vs,
            ve,
            CandidateValue::Text(ctx.text[vs..ve].to_string()),
        )
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const YEAR: i32 = 2026;

    fn scan(text: &str) -> Vec<Candidate> {
        scan_with(text, &[])
    }

    fn scan_with(text: &str, names: &[&str]) -> Vec<Candidate> {
        let found = find_candidates(&[SourceSegment { id: "s0", text }], names, YEAR);
        assert_invariants(&[text], &found);
        found
    }

    /// Every candidate is verbatim-traceable, typed and in document order.
    fn assert_invariants(texts: &[&str], found: &[Candidate]) {
        assert!(found.len() <= MAX_CANDIDATES);
        for (n, c) in found.iter().enumerate() {
            assert_eq!(c.id, format!("c{n}"));
            let seg: usize = c.segment_id[1..].parse().unwrap();
            let text = texts[seg];
            assert!(c.start < c.end, "{c:?}");
            assert!(text.is_char_boundary(c.start) && text.is_char_boundary(c.end));
            assert_eq!(&text[c.start..c.end], c.text);
            assert!(c.line.len() <= LINE_MAX);
            assert!(c.label.as_ref().is_none_or(|l| l.len() <= LABEL_MAX));
            match &c.value {
                CandidateValue::Date(date) => assert!(is_iso_date(date), "{date}"),
                CandidateValue::Money { amount, currency } => {
                    assert!(is_decimal(amount), "{amount}");
                    assert!(
                        currency.len() == 3 && currency.bytes().all(|b| b.is_ascii_uppercase())
                    );
                }
                CandidateValue::Amount(amount) => assert!(is_decimal(amount), "{amount}"),
                CandidateValue::Period { start, end } => {
                    assert!(is_iso_date(start), "{start}");
                    assert!(is_iso_date(end), "{end}");
                    assert!(start <= end, "{start} / {end}");
                }
                // Plates keep their canonical "B-MX 1988" form.
                CandidateValue::Identifier(id) if c.kind != CandidateKind::Plate => {
                    assert!(!id.contains(' '), "{id}")
                }
                _ => {}
            }
        }
        for pair in found.windows(2) {
            let key = |c: &Candidate| (c.segment_id.clone(), c.start, c.kind, c.end);
            assert!(
                key(&pair[0]) < key(&pair[1]),
                "{:?} / {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    fn is_iso_date(s: &str) -> bool {
        let b = s.as_bytes();
        b.len() == 10
            && b[4] == b'-'
            && b[7] == b'-'
            && b.iter()
                .enumerate()
                .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
    }

    fn is_decimal(s: &str) -> bool {
        let s = s.strip_prefix('-').unwrap_or(s);
        let (int, frac) = s.split_once('.').unwrap_or((s, "0"));
        !int.is_empty()
            && !frac.is_empty()
            && int.bytes().all(|c| c.is_ascii_digit())
            && frac.bytes().all(|c| c.is_ascii_digit())
    }

    fn of_kind(found: &[Candidate], kind: CandidateKind) -> Vec<&Candidate> {
        found.iter().filter(|c| c.kind == kind).collect()
    }

    fn value_str(c: &Candidate) -> String {
        match &c.value {
            CandidateValue::Text(v) | CandidateValue::Identifier(v) | CandidateValue::Date(v) => {
                v.clone()
            }
            CandidateValue::Money { amount, currency } => format!("{amount} {currency}"),
            CandidateValue::Amount(a) => a.clone(),
            CandidateValue::Period { start, end } => format!("{start} – {end}"),
            CandidateValue::Mrz(m) => m.document_number.clone(),
        }
    }

    fn values(found: &[Candidate], kind: CandidateKind) -> Vec<String> {
        of_kind(found, kind).into_iter().map(value_str).collect()
    }

    fn texts(found: &[Candidate], kind: CandidateKind) -> Vec<&str> {
        of_kind(found, kind)
            .into_iter()
            .map(|c| c.text.as_str())
            .collect()
    }

    // --- MRZ ----------------------------------------------------------------

    fn check(data: &str) -> char {
        const WEIGHTS: [u32; 3] = [7, 3, 1];
        let sum: u32 = data
            .bytes()
            .enumerate()
            .map(|(i, c)| {
                let v = match c {
                    b'0'..=b'9' => u32::from(c - b'0'),
                    b'A'..=b'Z' => u32::from(c - b'A') + 10,
                    _ => 0,
                };
                v * WEIGHTS[i % 3]
            })
            .sum();
        char::from(b'0' + (sum % 10) as u8)
    }

    fn pad(s: &str, width: usize) -> String {
        format!("{s:<<width$}")
    }

    fn td3(names: &str, number: &str, birth: &str, sex: char, expiry: &str, opt: &str) -> String {
        let l1 = pad(&format!("P<UTO{names}"), 44);
        let number = pad(number, 9);
        let opt = pad(opt, 14);
        let a = format!("{number}{}", check(&number));
        let b = format!("{birth}{}", check(birth));
        let c = format!("{expiry}{}", check(expiry));
        let o = format!("{opt}{}", check(&opt));
        let composite = check(&format!("{a}{b}{c}{o}"));
        let l2 = format!("{a}UTO{b}{sex}{c}{o}{composite}");
        assert_eq!((l1.len(), l2.len()), (44, 44));
        format!("{l1}\n{l2}")
    }

    fn td1(head: &str, number: &str, birth: &str, expiry: &str, nat: &str, names: &str) -> String {
        let number = pad(number, 9);
        let l1 = pad(&format!("{head}{number}{}", check(&number)), 30);
        let b = format!("{birth}{}", check(birth));
        let e = format!("{expiry}{}", check(expiry));
        let opt = "<".repeat(11);
        let composite = check(&format!("{}{b}{e}{opt}", &l1[5..30]));
        let l2 = format!("{b}F{e}{nat}{opt}{composite}");
        let l3 = pad(names, 30);
        assert_eq!((l1.len(), l2.len(), l3.len()), (30, 30, 30));
        format!("{l1}\n{l2}\n{l3}")
    }

    fn mrz_data(found: &[Candidate]) -> &MrzData {
        match &of_kind(found, CandidateKind::Mrz)[0].value {
            CandidateValue::Mrz(data) => data,
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn td3_builder_matches_icao_specimen() {
        let mrz = td3(
            "ERIKSSON<<ANNA<MARIA",
            "L898902C3",
            "740812",
            'F',
            "120415",
            "ZE184226B",
        );
        assert_eq!(
            mrz,
            "P<UTOERIKSSON<<ANNA<MARIA<<<<<<<<<<<<<<<<<<<\nL898902C36UTO7408122F1204159ZE184226B<<<<<10"
        );
        let td1 = td1(
            "I<UTO",
            "D23145890",
            "740812",
            "120415",
            "UTO",
            "ERIKSSON<<ANNA<MARIA",
        );
        assert_eq!(
            td1,
            "I<UTOD231458907<<<<<<<<<<<<<<<\n7408122F1204159UTO<<<<<<<<<<<6\nERIKSSON<<ANNA<MARIA<<<<<<<<<<"
        );
    }

    #[test]
    fn parses_valid_td3_with_sub_candidates() {
        let mrz = td3(
            "MUSTERMANN<<ERIKA",
            "C01X00T47",
            "830812",
            'F',
            "310101",
            "",
        );
        let text = format!("Reisepass / Passport\n{mrz}\n");
        let found = scan(&text);
        let data = mrz_data(&found);
        assert_eq!(data.format, "td3");
        assert_eq!(data.document_code, "P");
        assert_eq!(data.issuing_state, "UTO");
        assert_eq!(
            (data.surname.as_str(), data.given_names.as_str()),
            ("MUSTERMANN", "ERIKA")
        );
        assert_eq!(data.document_number, "C01X00T47");
        assert_eq!(data.nationality, "UTO");
        assert_eq!(data.birth_date.as_deref(), Some("1983-08-12"));
        assert_eq!(data.expiry_date.as_deref(), Some("2031-01-01"));
        assert_eq!(data.sex, "F");
        assert!(data.check_digits_valid);
        let mrz_candidate = of_kind(&found, CandidateKind::Mrz)[0];
        assert!(mrz_candidate.checksum);
        assert_eq!(mrz_candidate.text, mrz);
        let number = of_kind(&found, CandidateKind::Identifier)
            .into_iter()
            .find(|c| c.text == "C01X00T47")
            .expect("document number");
        assert!(number.checksum);
        let dates = of_kind(&found, CandidateKind::Date);
        assert_eq!(
            dates
                .iter()
                .map(|c| (c.text.as_str(), value_str(c), c.checksum))
                .collect::<Vec<_>>(),
            [
                ("830812", "1983-08-12".to_string(), true),
                ("310101", "2031-01-01".to_string(), true)
            ]
        );
    }

    #[test]
    fn normalizes_ocr_noise_in_mrz() {
        let mrz = td3("MUSTERMANN<<MAX", "C01X00T47", "050101", 'M', "300101", "");
        let noisy = mrz
            .replace('<', "«")
            .replacen("MUSTERMANN", "MUSTER MANN", 1)
            .to_lowercase();
        let text = format!("  {noisy}  ");
        let found = scan(&text);
        let data = mrz_data(&found);
        assert!(data.check_digits_valid);
        assert_eq!(data.surname, "MUSTER MANN".replace(' ', ""));
        assert_eq!(data.birth_date.as_deref(), Some("2005-01-01"));
        let candidate = of_kind(&found, CandidateKind::Mrz)[0];
        assert_eq!(candidate.text, noisy.trim());
        let number = of_kind(&found, CandidateKind::Identifier)[0];
        assert_eq!(number.text, "c01x00t47");
        assert_eq!(value_str(number), "C01X00T47");
    }

    #[test]
    fn mrz_with_bad_check_digit_is_kept_but_not_validated() {
        let mrz = td3(
            "MUSTERMANN<<ERIKA",
            "C01X00T47",
            "830812",
            'F',
            "310101",
            "",
        );
        let broken = mrz.replacen("C01X00T47", "C01X00T48", 1);
        let found = scan(&broken);
        let data = mrz_data(&found);
        assert!(!data.check_digits_valid);
        assert!(!of_kind(&found, CandidateKind::Mrz)[0].checksum);
        let number = of_kind(&found, CandidateKind::Identifier)[0];
        assert!(!number.checksum);
        // Dates keep their own (still valid) check digits.
        assert!(
            of_kind(&found, CandidateKind::Date)
                .iter()
                .all(|c| c.checksum)
        );
    }

    #[test]
    fn parses_td1_identity_card_and_birth_century() {
        let mrz = td1(
            "IDD<<",
            "T22000129",
            "300101",
            "310331",
            "D<<",
            "MUSTERMANN<<ERIKA",
        );
        let found = scan(&mrz);
        let data = mrz_data(&found);
        assert_eq!(data.format, "td1");
        assert_eq!(data.document_code, "ID");
        assert_eq!(data.issuing_state, "D");
        assert_eq!(data.nationality, "D");
        assert_eq!(data.document_number, "T22000129");
        assert_eq!(data.birth_date.as_deref(), Some("1930-01-01"));
        assert_eq!(data.expiry_date.as_deref(), Some("2031-03-31"));
        assert_eq!(
            (data.surname.as_str(), data.given_names.as_str()),
            ("MUSTERMANN", "ERIKA")
        );
        assert!(data.check_digits_valid);
        assert_eq!(of_kind(&found, CandidateKind::Mrz)[0].text, mrz);
        assert_eq!(values(&found, CandidateKind::Identifier), ["T22000129"]);
    }

    #[test]
    fn parses_td2() {
        let number = "D23145890";
        let a = format!("{number}{}", check(number));
        let b = format!("740812{}", check("740812"));
        let c = format!("120415{}", check("120415"));
        let opt = "<".repeat(7);
        let composite = check(&format!("{a}{b}{c}{opt}"));
        let l1 = pad("I<UTOERIKSSON<<ANNA<MARIA", 36);
        let l2 = format!("{a}UTO{b}F{c}{opt}{composite}");
        let found = scan(&format!("{l1}\n{l2}"));
        let data = mrz_data(&found);
        assert_eq!(data.format, "td2");
        assert_eq!(data.given_names, "ANNA MARIA");
        assert!(data.check_digits_valid);
    }

    #[test]
    fn mrz_check_digit_matches_icao_examples() {
        assert_eq!(mrz_check_digit(b"L898902C3"), 6);
        assert_eq!(mrz_check_digit(b"740812"), 2);
        assert_eq!(mrz_check_digit(b"120415"), 9);
    }

    // --- IBAN / BIC ---------------------------------------------------------

    #[test]
    fn finds_valid_ibans_grouped_and_compact() {
        let text = "IBAN: DE89 3704 0044 0532 0130 00 BIC: COBADEFFXXX\n\
            Kontonummer (IBAN): DE89370400440532013000.";
        let found = scan(text);
        let ibans = of_kind(&found, CandidateKind::Iban);
        assert_eq!(ibans.len(), 2);
        assert_eq!(ibans[0].text, "DE89 3704 0044 0532 0130 00");
        assert!(
            ibans
                .iter()
                .all(|c| c.checksum && value_str(c) == "DE89370400440532013000")
        );
        assert_eq!(ibans[0].label.as_deref(), Some("IBAN"));
        assert_eq!(values(&found, CandidateKind::Bic), ["COBADEFFXXX"]);
        assert!(of_kind(&found, CandidateKind::Phone).is_empty());
        assert!(of_kind(&found, CandidateKind::TaxId).is_empty());
        assert!(of_kind(&found, CandidateKind::Identifier).is_empty());
    }

    #[test]
    fn rejects_invalid_ibans_and_accepts_other_countries() {
        let found = scan("IBAN: DE89 3704 0044 0532 0130 01\nIBAN DE89 3704 0044 0532 0130");
        assert!(of_kind(&found, CandidateKind::Iban).is_empty());
        // Country without a length entry: any valid length 15..=34 is accepted.
        let other = iban("XK", "1212012345678906");
        let grouped: Vec<String> = other
            .as_bytes()
            .chunks(4)
            .map(|c| String::from_utf8_lossy(c).into_owned())
            .collect();
        let text = format!("GB82 WEST 1234 5698 7654 32 and {}", grouped.join(" "));
        let found = scan(&text);
        assert_eq!(
            values(&found, CandidateKind::Iban),
            ["GB82WEST12345698765432", other.as_str()]
        );
        // Known country with the wrong length.
        let short = iban("DE", "3704004405320130");
        assert!(of_kind(&scan(&short), CandidateKind::Iban).is_empty());
    }

    fn iban(country: &str, bban: &str) -> String {
        let rearranged = format!("{bban}{country}00");
        let mut rest = 0u32;
        for c in rearranged.bytes() {
            rest = match c {
                b'0'..=b'9' => (rest * 10 + u32::from(c - b'0')) % 97,
                _ => (rest * 100 + u32::from(c - b'A') + 10) % 97,
            };
        }
        format!("{country}{:02}{bban}", 98 - rest)
    }

    #[test]
    fn bic_requires_context() {
        assert!(of_kind(&scan("Betreff COBADEFFXXX"), CandidateKind::Bic).is_empty());
        assert_eq!(
            values(&scan("SWIFT-Code DEUTDEFF"), CandidateKind::Bic),
            ["DEUTDEFF"]
        );
    }

    // --- Steuer-ID ----------------------------------------------------------

    fn tax_id(first10: &str) -> String {
        let digits: Vec<u8> = first10.bytes().map(|c| c - b'0').collect();
        format!("{first10}{}", mod11_10_check(&digits))
    }

    #[test]
    fn validates_tax_id_rules() {
        assert!(tax_id_valid(&compact_digits("86095742719")));
        assert!(!tax_id_valid(&compact_digits("86095742718")));
        // One digit three times, not consecutive: valid.
        assert!(tax_id_valid(&compact_digits(&tax_id("1121345678"))));
        // Three consecutive identical digits: invalid.
        assert!(!tax_id_valid(&compact_digits(&tax_id("1112345678"))));
        // All ten digits distinct: invalid.
        assert!(!tax_id_valid(&compact_digits(&tax_id("1234567890"))));
        // Two repeated digits: invalid.
        assert!(!tax_id_valid(&compact_digits(&tax_id("1122345678"))));
        // Leading zero: invalid.
        assert!(!tax_id_valid(&compact_digits(&tax_id("0123456789"))));
    }

    #[test]
    fn finds_tax_ids_in_text() {
        let spaced = tax_id("5732981405");
        assert!(tax_id_valid(&compact_digits(&spaced)), "{spaced}");
        let grouped = format!(
            "{} {} {} {}",
            &spaced[..2],
            &spaced[2..5],
            &spaced[5..8],
            &spaced[8..]
        );
        let text = format!(
            "Identifikationsnummer: {grouped}\nIdNr 86095742719\nReferenz 86095742718 ohne Bezug"
        );
        let found = scan(&text);
        let tax = of_kind(&found, CandidateKind::TaxId);
        assert_eq!(tax.len(), 2);
        assert_eq!(tax[0].text, grouped);
        assert_eq!(value_str(tax[0]), spaced);
        assert_eq!(tax[0].label.as_deref(), Some("Identifikationsnummer"));
        assert!(tax.iter().all(|c| c.checksum));
        assert_eq!(value_str(tax[1]), "86095742719");
        // Invalid numbers are not tax IDs.
        assert!(!values(&found, CandidateKind::TaxId).contains(&"86095742718".to_string()));
    }

    #[test]
    fn invalid_tax_id_near_label_becomes_unchecked_identifier() {
        let found = scan("Steuerliche Identifikationsnummer 86 095 742 718");
        assert!(of_kind(&found, CandidateKind::TaxId).is_empty());
        let ids = of_kind(&found, CandidateKind::Identifier);
        assert_eq!(ids.len(), 1);
        assert_eq!(value_str(ids[0]), "86095742718");
        assert!(!ids[0].checksum);
        assert!(
            scan("Zahl 86095742718")
                .iter()
                .all(|c| c.kind != CandidateKind::TaxId)
        );
    }

    // --- Rentenversicherungsnummer -----------------------------------------

    #[test]
    fn finds_social_insurance_numbers() {
        assert_eq!(social_check(&[6, 5, 1, 7, 0, 8, 3, 9], b'J', &[0, 0]), 3);
        let text = "Versicherungsnummer: 65 170839 J 003\nRV 65170839J003\nFalsch 65 170839 J 004";
        let found = scan(text);
        let numbers = of_kind(&found, CandidateKind::SocialInsuranceNumber);
        assert_eq!(numbers.len(), 2);
        assert_eq!(numbers[0].text, "65 170839 J 003");
        assert!(
            numbers
                .iter()
                .all(|c| c.checksum && value_str(c) == "65170839J003")
        );
        let flagged = scan("Rentenversicherungsnummer 65 170839 J 004");
        let numbers = of_kind(&flagged, CandidateKind::SocialInsuranceNumber);
        assert_eq!(numbers.len(), 1);
        assert!(!numbers[0].checksum);
    }

    // --- Dates --------------------------------------------------------------

    #[test]
    fn finds_dates_in_all_supported_formats() {
        let text = "Geburtsdatum: 14.03.1988\n\
            am 14.3.1988, 14. März 1988, 14. Maerz 1988 und 3. Jänner 2020\n\
            14 March 1988 / March 14, 1988 / 1988-03-14 / 14/03/1988\n\
            29.02.2024 und 14. MÄRZ 1988";
        let found = scan(text);
        let dates = values(&found, CandidateKind::Date);
        let expected = [
            "1988-03-14",
            "1988-03-14",
            "1988-03-14",
            "1988-03-14",
            "2020-01-03",
            "1988-03-14",
            "1988-03-14",
            "1988-03-14",
            "1988-03-14",
            "2024-02-29",
            "1988-03-14",
        ];
        assert_eq!(dates, expected);
        assert_eq!(
            texts(&found, CandidateKind::Date)[..5],
            [
                "14.03.1988",
                "14.3.1988",
                "14. März 1988",
                "14. Maerz 1988",
                "3. Jänner 2020"
            ]
        );
    }

    #[test]
    fn never_invents_centuries_or_impossible_dates() {
        let text = "14.03.88 30.02.2024 29.02.2023 03/2024 März 2024 32.01.2020 14.13.2020 1.2.3";
        assert!(of_kind(&scan(text), CandidateKind::Date).is_empty());
        let found = scan("Gültig bis: März 2024");
        assert!(of_kind(&found, CandidateKind::Date).is_empty());
        // "März 2024" is now a complete month period, not a generic label/value pair.
        assert!(of_kind(&found, CandidateKind::LabelValue).is_empty());
        let periods = of_kind(&found, CandidateKind::Period);
        assert_eq!(value_str(periods[0]), "2024-03-01 – 2024-03-31");
        assert_eq!(periods[0].label.as_deref(), Some("Gültig bis"));
    }

    // --- Money --------------------------------------------------------------

    #[test]
    fn finds_money_with_explicit_currency() {
        let lines = [
            ("1.234,56 €", "1234.56 EUR"),
            ("1.234,56 EUR", "1234.56 EUR"),
            ("EUR 1.234,56", "1234.56 EUR"),
            ("€ 1.234,56", "1234.56 EUR"),
            ("-12,50 €", "-12.50 EUR"),
            ("1234,56 €", "1234.56 EUR"),
            ("1,234.56 USD", "1234.56 USD"),
            ("$1,234.56", "1234.56 USD"),
            ("CHF 1'234.50", "1234.50 CHF"),
            ("£12.00", "12.00 GBP"),
            ("500,- Euro", "500 EUR"),
            ("0,99€", "0.99 EUR"),
        ];
        let text = lines.map(|(line, _)| line).join("\n");
        let found = scan(&text);
        assert_eq!(
            texts(&found, CandidateKind::Money),
            lines.map(|(line, _)| line)
        );
        assert_eq!(
            values(&found, CandidateKind::Money),
            lines.map(|(_, value)| value)
        );
    }

    #[test]
    fn amounts_without_currency_are_not_money() {
        let found = scan("Summe 1.234,56\nZins 12,5 %\nDatum 01.02.2024 EUR-Konto\n10-20 €");
        assert!(of_kind(&found, CandidateKind::Money).is_empty());
    }

    // --- Bare amounts and periods --------------------------------------------

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
        assert_eq!(
            labeled_amount(&found, "Gesamt-Brutto").as_deref(),
            Some("5340.00")
        );
        assert_eq!(
            labeled_amount(&found, "Lohnsteuer").as_deref(),
            Some("1032.58")
        );
        assert_eq!(
            labeled_amount(&found, "Netto-Verdienst").as_deref(),
            Some("3210.05")
        );
        assert_eq!(
            labeled_amount(&found, "Solidaritätszuschlag").as_deref(),
            Some("0.00")
        );
        // A currency beside the value stays money and is never duplicated as an amount.
        assert!(
            found
                .iter()
                .any(|c| c.kind == CandidateKind::Money && c.text == "3.210,05 EUR")
        );
        assert!(
            !found
                .iter()
                .any(|c| c.kind == CandidateKind::Amount && c.text.contains("EUR"))
        );
        // Percentages and three-decimal factors are not amounts.
        assert!(
            !found
                .iter()
                .any(|c| c.kind == CandidateKind::Amount && c.text == "2,50")
        );
        assert!(
            !found
                .iter()
                .any(|c| c.kind == CandidateKind::Amount && c.text == "0,950")
        );
        // The label/value catch-all at the same span hands its label over.
        assert!(
            !found
                .iter()
                .any(|c| c.kind == CandidateKind::LabelValue && c.text == "1.032,58")
        );
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
        assert_eq!(
            parse_bare_amount(" 1.032,58 ", false, false).as_deref(),
            Some("1032.58")
        );
        assert_eq!(parse_bare_amount("4.200", false, false), None);
        assert_eq!(
            parse_bare_amount("4.200", false, true).as_deref(),
            Some("4200")
        );
        assert_eq!(
            parse_bare_amount("64.080   00", true, false).as_deref(),
            Some("64080.00")
        );
        assert_eq!(parse_bare_amount("64.080   00", false, false), None);
        assert_eq!(parse_bare_amount("1.032,58 Euro", false, true), None);
        assert_eq!(
            parse_bare_amount("-12,50", false, false).as_deref(),
            Some("-12.50")
        );
    }

    #[test]
    fn currencies_in_finds_every_explicit_marker_once() {
        assert_eq!(
            currencies_in("1.234,56 € and 12,50 USD and 500 Euro, again in EUR"),
            std::collections::BTreeSet::from(["EUR", "USD"])
        );
        assert!(currencies_in("Summe 1.234,56").is_empty());
    }

    // --- Identifiers --------------------------------------------------------

    #[test]
    fn finds_labeled_identifiers() {
        let text = "Vertragsnummer: KV 123 456 789 vom 01.02.2024\n\
            Kunden-Nr. 4711-0815\n\
            Aktenzeichen: 12 O 345/23\n\
            Versicherungsschein-Nr.: VS-2024/00017\n\
            Mitgliedsnummer\n  A 123 456 789\n\
            Az. 3 K 12/24 B vom Gericht\n\
            Kundennummer: siehe Anlage";
        let found = scan(text);
        let ids: Vec<(String, Option<String>, bool)> = of_kind(&found, CandidateKind::Identifier)
            .into_iter()
            .map(|c| (value_str(c), c.label.clone(), c.checksum))
            .collect();
        let expected = [
            ("KV123456789", "Vertragsnummer"),
            ("4711-0815", "Kunden-Nr."),
            ("12O345/23", "Aktenzeichen"),
            ("VS-2024/00017", "Versicherungsschein-Nr."),
            ("A123456789", "Mitgliedsnummer"),
            ("3K12/24", "Az."),
        ]
        .map(|(v, l)| (v.to_string(), Some(l.to_string()), false));
        assert_eq!(ids, expected);
        assert_eq!(
            of_kind(&found, CandidateKind::Identifier)[0].text,
            "KV 123 456 789"
        );
    }

    // --- Names --------------------------------------------------------------

    #[test]
    fn finds_known_names_in_all_orders_and_spellings() {
        let text = "Max Mustermann\nMUSTERMANN, MAX\nMustermann Max\nJuergen Mueller\n\
            MÜLLER JÜRGEN\nMueller,Juergen\nMaxi Mustermannsen\nMUSTERMANN<<MAX<<<<<<<<";
        let found = scan_with(text, &["Max Mustermann", "Jürgen Müller", "max mustermann"]);
        assert_eq!(
            texts(&found, CandidateKind::PersonName),
            [
                "Max Mustermann",
                "MUSTERMANN, MAX",
                "Mustermann Max",
                "Juergen Mueller",
                "MÜLLER JÜRGEN",
                "Mueller,Juergen",
                "MUSTERMANN<<MAX"
            ]
        );
        let values = values(&found, CandidateKind::PersonName);
        assert_eq!(values[0], "Max Mustermann");
        assert_eq!(values[4], "Jürgen Müller");
    }

    #[test]
    fn finds_names_after_salutations_and_labels() {
        let text = "Sehr geehrter Herr Dr. Max Mustermann,\n\
            Frau Erika Mustermann-Gabler\n\
            Vorname: Max  Nachname: Mustermann\n\
            Kontoinhaber: Erika Mustermann\n\
            Herr und Frau Mustermann\n\
            die Frau des Hauses";
        let found = scan(text);
        assert_eq!(
            texts(&found, CandidateKind::PersonName),
            [
                "Max Mustermann",
                "Erika Mustermann-Gabler",
                "Max",
                "Mustermann",
                "Erika Mustermann",
                "Mustermann"
            ]
        );
        assert!(of_kind(&found, CandidateKind::LabelValue).is_empty());
    }

    // --- Plates and VINs ----------------------------------------------------

    #[test]
    fn finds_german_plates() {
        let text = "Amtliches Kennzeichen: B MX 1988\nM-AB 123E\nWeg B MX 1988 ohne Kontext\n\
            Kfz: ÖHR-AB 12H\nISO-IEC 7064";
        let found = scan(text);
        assert_eq!(
            values(&found, CandidateKind::Plate),
            ["B-MX 1988", "M-AB 123E", "ÖHR-AB 12H"]
        );
        assert_eq!(texts(&found, CandidateKind::Plate)[0], "B MX 1988");
    }

    #[test]
    fn finds_vins_only_in_context() {
        let text = "FIN: WVWZZZ1JZXW000001\n\
            Fahrzeug-Identifizierungsnummer wvwzzz1jzxw000002\n\
            Bestellung WVWZZZ1JZXW000003\n\
            VIN WVWZZZ1JZOW000004";
        let found = scan(text);
        assert_eq!(
            values(&found, CandidateKind::Vin),
            ["WVWZZZ1JZXW000001", "WVWZZZ1JZXW000002"]
        );
    }

    // --- Email and phone ----------------------------------------------------

    #[test]
    fn finds_emails_and_phone_numbers() {
        let text = "E-Mail: max.mustermann@example.org\n\
            Tel.: +49 30 1234567\n\
            Telefon (030) 123 45 67\n\
            Mobil: +49 (0) 171 2345678\n\
            0171 2345678\n\
            Rechnung 0301234567\n\
            kein@mail und a@b.c";
        let found = scan(text);
        assert_eq!(
            values(&found, CandidateKind::Email),
            ["max.mustermann@example.org"]
        );
        assert_eq!(
            values(&found, CandidateKind::Phone),
            ["+49301234567", "0301234567", "+491712345678", "01712345678"]
        );
        assert_eq!(texts(&found, CandidateKind::Phone)[1], "(030) 123 45 67");
        assert!(of_kind(&found, CandidateKind::LabelValue).is_empty());
    }

    // --- Label/value and labels --------------------------------------------

    #[test]
    fn finds_label_values_without_duplicating_specific_candidates() {
        let text = "Geburtsort: Musterstadt\nGeburtsdatum: 14.03.1988\nGültig bis\t03/2024\n\
            Name  Wert: x\nZeit 12:30 Uhr\nhttps://example.org";
        let found = scan(text);
        let pairs: Vec<(String, Option<String>)> = of_kind(&found, CandidateKind::LabelValue)
            .into_iter()
            .map(|c| (value_str(c), c.label.clone()))
            .collect();
        let expected = [("Musterstadt", "Geburtsort"), ("x", "Wert")]
            .map(|(v, l)| (v.to_string(), Some(l.to_string())));
        assert_eq!(pairs, expected);
        assert_eq!(values(&found, CandidateKind::Date), ["1988-03-14"]);
        // "03/2024" is now a complete month period, which hands over the label
        // that would otherwise have made it a label/value pair.
        let periods = of_kind(&found, CandidateKind::Period);
        assert_eq!(value_str(periods[0]), "2024-03-01 – 2024-03-31");
        assert_eq!(periods[0].label.as_deref(), Some("Gültig bis"));
    }

    #[test]
    fn detects_labels_on_same_and_previous_lines() {
        let text = "Versicherungsbeginn:\n01.04.2024\nBetrag 12,50 €\nZahlbar bis\n15.05.2024\n\
            Summe 99\n16.05.2024\nName: Max  Geburtsdatum: 14.03.1988";
        let found = scan(text);
        let labels: Vec<Option<&str>> = of_kind(&found, CandidateKind::Date)
            .into_iter()
            .map(|c| c.label.as_deref())
            .collect();
        assert_eq!(
            labels,
            [
                Some("Versicherungsbeginn"),
                Some("Zahlbar bis"),
                None,
                Some("Geburtsdatum")
            ]
        );
        assert_eq!(of_kind(&found, CandidateKind::Money)[0].label, None);
        let tabular = scan("Versicherungsbeginn    01.02.2024\nBeitrag\t89,90 €");
        let labels: Vec<Option<&str>> = tabular.iter().map(|c| c.label.as_deref()).collect();
        assert_eq!(labels, [Some("Versicherungsbeginn"), Some("Beitrag")]);
    }

    #[test]
    fn windows_long_lines_around_the_candidate() {
        let text = format!("{} 14.03.1988 {}", "x".repeat(1000), "ü".repeat(1000));
        let found = scan(&text);
        let date = of_kind(&found, CandidateKind::Date)[0];
        assert!(date.line.contains("14.03.1988"));
        assert!(date.line.len() <= LINE_MAX);
        let short = scan("  Datum:\t14.03.1988  \r\n");
        assert_eq!(short[0].line, "Datum:\t14.03.1988");
    }

    // --- Ordering, dedup, cap ----------------------------------------------

    #[test]
    fn numbers_candidates_across_segments_in_document_order() {
        let texts = [
            "Betrag: 12,50 €",
            "IBAN DE89370400440532013000\nam 01.02.2024",
        ];
        let segments = [
            SourceSegment {
                id: "s0",
                text: texts[0],
            },
            SourceSegment {
                id: "s1",
                text: texts[1],
            },
        ];
        let found = find_candidates(&segments, &[], YEAR);
        assert_invariants(&texts, &found);
        let summary: Vec<(&str, &str, CandidateKind)> = found
            .iter()
            .map(|c| (c.id.as_str(), c.segment_id.as_str(), c.kind))
            .collect();
        assert_eq!(
            summary,
            [
                ("c0", "s0", CandidateKind::Money),
                ("c1", "s1", CandidateKind::Iban),
                ("c2", "s1", CandidateKind::Date),
            ]
        );
    }

    #[test]
    fn caps_candidates_keeping_the_most_valuable() {
        let mut text = String::new();
        for i in 0..300 {
            text.push_str(&format!("Datum: {:02}.01.2000\n", i % 28 + 1));
        }
        text.push_str("DE89370400440532013000\n");
        let found = scan(&text);
        assert_eq!(found.len(), MAX_CANDIDATES);
        assert_eq!(found[MAX_CANDIDATES - 1].kind, CandidateKind::Iban);
        assert!(
            found[..MAX_CANDIDATES - 1]
                .iter()
                .all(|c| c.kind == CandidateKind::Date)
        );
        assert_eq!(found[0].start, 7);
    }

    // --- Robustness ---------------------------------------------------------

    #[test]
    fn never_panics_on_arbitrary_text() {
        assert!(scan("").is_empty());
        assert!(scan("\n\n  \r\n").is_empty());
        let palette = [
            'a', 'Z', 'ä', 'Ü', 'ß', '€', '£', '$', '<', '«', '‹', ':', ' ', ' ', '\t', '\n', '0',
            '1', '9', '7', '.', ',', '-', '/', '+', '@', '(', ')', '\'', '’', '−', '😀', '中',
            '\u{0}', '\u{a0}', 'D', 'E', 'I', 'B', 'A', 'N', 'P', 'J', 'H',
        ];
        let fragments = [
            "IBAN: DE89 3704 0044 0532 0130 00",
            "86 095 742 719",
            "65 170839 J 003",
            "14. März 1988",
            "1.234,56 €",
            "Herr Max Mustermann",
            "B-MX 1988",
            "P<UTOERIKSSON<<ANNA<MARIA<<<<<<<<<<<<<<<<<<<",
            "L898902C36UTO7408122F1204159ZE184226B<<<<<10",
            "Kundennummer: ",
            "max@example.org",
            "Tel. +49 30 1234567",
        ];
        let mut seed: u64 = 0x5eed;
        let mut next = move |n: usize| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((seed >> 33) as usize) % n
        };
        for _ in 0..400 {
            let mut text = String::new();
            for _ in 0..next(120) {
                if next(8) == 0 {
                    text.push_str(fragments[next(fragments.len())]);
                } else {
                    text.push(palette[next(palette.len())]);
                }
            }
            let found = find_candidates(
                &[SourceSegment {
                    id: "s0",
                    text: &text,
                }],
                &["Max Mustermann", "Ärger Ölmann", "中"],
                YEAR,
            );
            assert_invariants(&[&text], &found);
        }
    }

    #[test]
    fn handles_very_long_lines() {
        let texts = [
            "ä€1.234,56 € 0171 2345678 Kundennummer: 4711 ".repeat(5_000),
            "1".repeat(200_000),
            "@".repeat(100_000),
            "a@".repeat(50_000),
            "Herr ".repeat(50_000),
            "<".repeat(100_000),
            "1.".repeat(100_000),
        ];
        for text in &texts {
            let found = find_candidates(
                &[SourceSegment { id: "s0", text }],
                &["Max Mustermann"],
                YEAR,
            );
            assert_invariants(&[text], &found);
        }
    }

    // --- Serialization ------------------------------------------------------

    #[test]
    fn serializes_tagged_values() {
        let date = serde_json::to_value(CandidateValue::Date("1988-03-14".into())).unwrap();
        assert_eq!(
            date,
            serde_json::json!({"type": "date", "value": "1988-03-14"})
        );
        let money = serde_json::to_value(CandidateValue::Money {
            amount: "12.50".into(),
            currency: "EUR".into(),
        })
        .unwrap();
        assert_eq!(
            money,
            serde_json::json!({"type": "money", "value": {"amount": "12.50", "currency": "EUR"}})
        );
        assert_eq!(
            serde_json::to_value(CandidateKind::SocialInsuranceNumber).unwrap(),
            "social_insurance_number"
        );
        let found = scan("IBAN DE89370400440532013000");
        let json = serde_json::to_string(&found).unwrap();
        let back: Vec<Candidate> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, found);
    }
    #[test]
    fn organizations_and_german_addresses_are_found() {
        let text = "Acme Software GmbH & Co. KG\nArbeitgeber: Beispiel Versicherung AG\nHerrn Max Mustermann\nMusterstraße 12a\n10115 Berlin\nKarl-Marx-Allee 3, 80331 München\nTechniker Krankenkasse\n";
        let seg = [SourceSegment { id: "s", text }];
        let found = find_candidates(&seg, &[], 2026);
        let of = |kind: CandidateKind| -> Vec<String> {
            found
                .iter()
                .filter(|c| c.kind == kind)
                .map(|c| c.text.clone())
                .collect()
        };
        assert_eq!(
            of(CandidateKind::Organization),
            vec![
                "Acme Software GmbH & Co. KG",
                "Beispiel Versicherung AG",
                "Techniker Krankenkasse"
            ]
        );
        assert_eq!(
            of(CandidateKind::Street),
            vec!["Musterstraße 12a", "Karl-Marx-Allee 3"]
        );
        assert_eq!(of(CandidateKind::PostalCode), vec!["10115", "80331"]);
        assert_eq!(of(CandidateKind::City), vec!["Berlin", "München"]);
        for c in &found {
            assert_eq!(&text[c.start..c.end], c.text);
        }
    }
}
