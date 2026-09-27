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

// Interpretation guidance is local and reusable: private payroll data never enters
// a web search. See docs/document-processing.md for the DATEV reference sources.
pub(crate) const PAYROLL_GUIDE: &str = r#"
German payslip guide (apply only if the source is a Gehalts-/Lohn-/Entgeltabrechnung or Brutto/Netto-Abrechnung):
Inspect employee header, employer header, pay period, earnings table, tax/social-insurance calculation, net adjustments, bank footer and cumulative totals. Look for Personal-Nr./Pers.-Nr., Geburtsdatum/Geb.-Datum, Steuer-ID/IdNr, SV-Nr./Versicherungsnummer, Eintritt/Austritt, StKl, Faktor, Ki.Frbtr., Konfession, Freibeträge, Krankenkasse, KV-Zusatzbeitrag, PGRS/BGRS, Beitragsgruppe, Steuer-/SV-Tage, Kostenstelle, Arbeitszeit and Urlaub. Blank cells are not zero. DATEV headers may precede a separate values row; correlate columns carefully and do not shift a value into its neighbor. Geburtsdatum may be DDMMYY, which does not establish a century on its own.
Read all labeled wage components and quantities (Grundgehalt, Stundenlohn, Zulagen, Zuschläge, Bonus, Sachbezug, VWL). Separate Gesamt-Brutto, Steuer-Brutto and SV-Brutto. Separate Lohnsteuer, Kirchensteuer, Solidaritätszuschlag and the employee's KV/RV/AV/PV deductions from employer contributions. Netto-Verdienst and Auszahlungsbetrag are different fields; preserve both and any Netto-Bezüge/Netto-Abzüge/advance payments. Extract printed IBAN, BIC, bank and account holder. Jahreswerte/Verdienstbescheinigung are cumulative amounts, never monthly salary. This guide supplies meanings, never missing values.
Steuer-Brutto and SV-Brutto are not Gesamt-Brutto. A Nachberechnung or Korrektur month is its own period. Tag each monthly value with the checklist slot only when it is this payslip's own month; Jahreswerte, kumulierte Werte and Verdienstbescheinigung totals are slot none."#;

// General reading rules are context, never evidence for personal values.
pub(crate) const CORRESPONDENCE_GUIDE: &str = r#"
For letters and insurance/health-insurance correspondence, distinguish sender, recipient, insured person, policyholder, provider and payee. A sender's address or bank account is not the recipient's. Preserve policy/member/claim/reference numbers exactly, and distinguish issue date, coverage period, service date, due date and explicitly stated response deadline. Do not calculate relative deadlines, infer coverage, diagnose conditions, or turn generic policy conditions into facts about a person. Preserve negation, conditional language, approval versus rejection, pending versus paid, and requested versus established facts. Extract quoted personal correspondence separately from generic boilerplate.
For email, use the decoded envelope headers and body together; distinguish authored text, forwarded/quoted history and signatures. A From header is a claimed sender, not proof of authenticity. Do not assign a correspondent's details to the mailbox owner. Dates and facts in quoted replies belong to their original context. Attachments have their own evidence; never claim their content was read if only their names are supplied. Missing, truncated or ambiguous context must stay unknown. Public knowledge can explain terminology but can never supply absent personal facts.
"#;

const WAGE_TAX_CERTIFICATE_GUIDE: &str = r#"
Lohnsteuerbescheinigung (printout of the electronic wage tax certificate, BMF template). Rely on the printed labels; line numbers are hints and change between years.
Line 1 Bescheinigungszeitraum is the certificate period. Line 2 counts periods without wages (Anzahl "U") and letters S, M, F, FR.
Line 3 Bruttoarbeitslohn einschl. Sachbezüge. Line 4 Einbehaltene Lohnsteuer von 3. Line 5 Einbehaltener Solidaritätszuschlag von 3. Line 6 Kirchensteuer des Arbeitnehmers. Line 7 is the spouse's or partner's church tax: never the employee's.
Pension shares: line 22 a/b are the employer's shares; line 23 a/b are the employee's shares (23 a statutory pension). Line 24 a–c are tax-free employer subsidies. Line 25 employee statutory health insurance, line 26 employee care insurance, line 27 employee unemployment insurance. From 2026 line 28 is unbesetzt; earlier years used it for private insurance amounts.
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
    let kind = doc_type
        .and_then(doc_types::doc_type)
        .filter(|k| k.tier == doc_types::Tier::Eager);
    let mut text = String::new();
    for part in [
        family_text(family),
        doc_type.map_or("", type_text),
        CORRESPONDENCE_GUIDE,
    ] {
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
        key: format!(
            "{family}|{}|{}",
            doc_type.unwrap_or("-"),
            doc_types::REGISTRY_VERSION
        ),
        text,
        checklist,
    }
}

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
        assert!(
            !keys.contains(&"period_start".to_owned()) || keys.contains(&"pay_month".to_owned())
        );
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
