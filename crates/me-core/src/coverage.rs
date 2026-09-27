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
        Money | Amount | Date | Period => c.label.is_some(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SourceSegment, find_candidates, locate};

    const PAYSLIP: &str = include_str!("../fixtures/documents/payslip_datev_rows.txt");

    #[test]
    fn values_no_fact_covers_are_listed_and_covered_ones_are_not() {
        let segments = [SourceSegment {
            id: "s0",
            text: PAYSLIP,
        }];
        let found = find_candidates(&segments, &[], 2026);
        let span = |quote: &str| {
            let at = locate(&segments, "s0", quote, quote).unwrap();
            ("s0".to_owned(), at.quote_start, at.quote_end)
        };
        let covered = vec![
            span("Gesamt-Brutto   5.340,00"),
            span("Lohnsteuer   1.032,58"),
        ];
        let open = uncovered(&found, &covered);
        let texts: Vec<&str> = open.iter().map(|u| u.text.as_str()).collect();
        assert!(
            texts.contains(&"3.210,05"),
            "net pay must be listed: {texts:?}"
        );
        assert!(texts.contains(&"435,21"));
        assert!(
            texts.contains(&"65929970489"),
            "checksum tax ID must be listed"
        );
        // No uncovered item lies inside a covered quote span.
        assert!(open.iter().all(|u| {
            !covered
                .iter()
                .any(|(s, a, b)| *s == u.segment_id && *a <= u.start && u.end <= *b)
        }));
        // Unlabeled or free-text candidates are not audit material.
        assert!(open.iter().all(|u| u.label.is_some()
            || matches!(
                u.kind,
                CandidateKind::TaxId
                    | CandidateKind::Iban
                    | CandidateKind::SocialInsuranceNumber
                    | CandidateKind::Mrz
            )));
    }
}
