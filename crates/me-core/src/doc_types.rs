//! Versioned registry of personal document types. A type declares its tier, the
//! values (slots) worth reading, the entities it yields and how they link. Adding
//! a type adds data here, not a pipeline path. TypeSafe question wording lives in
//! `me-agent`; this registry only supplies field names and descriptions.
use crate::{CandidateKind as C, EntityKind, Period};
use serde::{Deserialize, Serialize};

/// Changing any type, slot or link changes this version and reruns later stages.
pub const REGISTRY_VERSION: &str = "doc-types-v2";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Extracted during import.
    Eager,
    /// Classified and searchable; extracted when a question or task needs it.
    Lazy,
}
impl Tier {
    pub fn key(self) -> &'static str {
        match self {
            Self::Eager => "eager",
            Self::Lazy => "lazy",
        }
    }
}

pub struct Family {
    pub key: &'static str,
    pub label: &'static str,
    /// What belongs here; used as the Choice option description.
    pub what: &'static str,
    /// Boundary cases that belong elsewhere.
    pub not_for: &'static str,
}

/// Which entity a slot value or link end belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The person the document is about (an identity anchor), if known.
    Subject,
    /// An entity declared by the document type.
    Role(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueKind {
    Date,
    /// A fixed recurrence, or `None` when the period is asked as a closed Choice.
    Money(Option<Period>),
    Identifier,
    Text,
    /// Names an entity (its label); not stored as an assertion value.
    Name,
    /// A closed set judged by TypeSafe from the document, not a printed span.
    Category(&'static [(&'static str, &'static str)]),
    /// A month or year the document covers; code computes its first and last day.
    Period,
    /// A signed one-off amount whose direction (refund or payment) TypeSafe judges;
    /// code stores a refund as negative.
    Balance,
}

/// Validity of the assertion a slot or link produces. Computed in code from dates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotValidity {
    /// Identity attributes such as an IBAN or a passport number.
    Timeless,
    /// The statement period (`period_start` .. `period_end` inclusive).
    DocumentPeriod,
    /// Open interval from a date slot; falls back to the document date.
    From(&'static str),
}

/// Machine-readable-zone field whose position defines its meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MrzField {
    DocumentNumber,
    BirthDate,
    ExpiryDate,
    Nationality,
}

pub struct Slot {
    pub key: &'static str,
    /// Vocabulary key. `None` for meta slots (document date, period) and names.
    pub property: Option<&'static str>,
    pub target: Target,
    pub value: ValueKind,
    /// Candidate kinds offered to the slot Choice.
    pub accepts: &'static [C],
    pub label: &'static str,
    pub description: &'static str,
    pub required: bool,
    pub validity: SlotValidity,
    pub mrz: Option<MrzField>,
}

pub struct IdentifierSpec {
    pub namespace: &'static str,
    pub slot: &'static str,
    /// A slot whose normalized value scopes the identifier (e.g. the insurer).
    pub scope_slot: Option<&'static str>,
}

pub struct EntitySpec {
    pub role: &'static str,
    pub kind: EntityKind,
    pub label_slot: Option<&'static str>,
    pub fallback_label: &'static str,
    pub identifiers: &'static [IdentifierSpec],
    /// Fixed facts, e.g. the kind of identity document.
    pub constants: &'static [(&'static str, &'static str)],
}

pub struct Link {
    pub from: Target,
    pub property: &'static str,
    pub to: Target,
    pub validity: SlotValidity,
}

pub struct DocType {
    pub key: &'static str,
    pub family: &'static str,
    pub label: &'static str,
    pub what: &'static str,
    pub tier: Tier,
    pub entities: &'static [EntitySpec],
    pub slots: &'static [Slot],
    pub links: &'static [Link],
}

pub const FAMILIES: &[Family] = &[
    Family {
        key: "identity",
        label: "Identity",
        what: "Official identity documents: passport, national ID card, driving licence, residence permit",
        not_for: "Letters that merely mention an ID number",
    },
    Family {
        key: "tax",
        label: "Tax",
        what: "Tax office documents and tax certificates: Lohnsteuerbescheinigung, Steuerbescheid, tax return, donation receipt",
        not_for: "Payslips, invoices showing VAT",
    },
    Family {
        key: "employment",
        label: "Employment",
        what: "Payslips, employment contracts, references and other documents from an employer",
        not_for: "Annual wage tax certificates",
    },
    Family {
        key: "insurance",
        label: "Insurance",
        what: "Insurance policies, health insurance notices, premium notices and claim letters",
        not_for: "Bank statements that list an insurance payment",
    },
    Family {
        key: "banking",
        label: "Banking",
        what: "Bank statements, account opening documents, card and loan letters from a bank",
        not_for: "Invoices that print the payee's bank details",
    },
    Family {
        key: "vehicle",
        label: "Vehicle",
        what: "Vehicle registration certificates (Zulassungsbescheinigung), vehicle purchase and service documents",
        not_for: "Car insurance policies",
    },
    Family {
        key: "housing",
        label: "Housing",
        what: "Rental contracts, residence registration (Meldebescheinigung), utility and service charge statements",
        not_for: "Letters that only print an address",
    },
    Family {
        key: "contract",
        label: "Contract",
        what: "Contracts with service providers: phone, internet, energy, gym, subscriptions",
        not_for: "Employment, rental and insurance contracts",
    },
    Family {
        key: "health",
        label: "Health",
        what: "Medical reports, prescriptions, sick notes and treatment documents",
        not_for: "Health insurance membership and premium letters",
    },
    Family {
        key: "invoice",
        label: "Invoices",
        what: "Invoices, bills, receipts and payment reminders",
        not_for: "Contracts and bank statements",
    },
    Family {
        key: "correspondence",
        label: "Correspondence",
        what: "Other personal or administrative letters and emails",
        not_for: "Documents of a more specific family",
    },
    Family {
        key: "noise",
        label: "Advertising",
        what: "Advertising, newsletters, marketing flyers and other material without personal records",
        not_for: "Letters about an existing personal contract",
    },
    Family {
        key: "other",
        label: "Other",
        what: "Anything else, or several unrelated documents in one file",
        not_for: "",
    },
];

const DATE: &[C] = &[C::Date];
const MONEY: &[C] = &[C::Money, C::Amount];
const ORG: &[C] = &[C::Organization, C::LabelValue];
const IDENT: &[C] = &[C::Identifier, C::LabelValue];
const TEXT: &[C] = &[C::LabelValue];
const PERIOD: &[C] = &[C::Period];

/// Wage tax classes; printed as `1`–`6` or Roman numerals.
pub const TAX_CLASSES: &[(&str, &str)] = &[
    (
        "I",
        "Steuerklasse I: single, or separated/divorced/widowed without the relief",
    ),
    (
        "II",
        "Steuerklasse II: single parent with the relief amount",
    ),
    (
        "III",
        "Steuerklasse III: married, the higher-earning partner",
    ),
    (
        "IV",
        "Steuerklasse IV: married, both partners earning similarly (also with factor)",
    ),
    (
        "V",
        "Steuerklasse V: married, the partner of someone in class III",
    ),
    ("VI", "Steuerklasse VI: a second or further employment"),
];

/// Legal currency of German payroll and tax forms, used only when the document
/// itself prints no single currency.
pub fn default_currency(doc_type: &str) -> Option<&'static str> {
    matches!(
        doc_type,
        "payslip" | "wage_tax_certificate" | "tax_assessment"
    )
    .then_some("EUR")
}

const fn employee_money(
    key: &'static str,
    property: &'static str,
    period: Period,
    label: &'static str,
    description: &'static str,
) -> Slot {
    valid(
        slot(
            key,
            Some(property),
            Target::Subject,
            ValueKind::Money(Some(period)),
            MONEY,
            label,
            description,
        ),
        SlotValidity::DocumentPeriod,
    )
}

const fn slot(
    key: &'static str,
    property: Option<&'static str>,
    target: Target,
    value: ValueKind,
    accepts: &'static [C],
    label: &'static str,
    description: &'static str,
) -> Slot {
    Slot {
        key,
        property,
        target,
        value,
        accepts,
        label,
        description,
        required: false,
        validity: SlotValidity::Timeless,
        mrz: None,
    }
}
const fn required(mut s: Slot) -> Slot {
    s.required = true;
    s
}
const fn valid(mut s: Slot, validity: SlotValidity) -> Slot {
    s.validity = validity;
    s
}
const fn mrz(mut s: Slot, field: MrzField) -> Slot {
    s.mrz = Some(field);
    s
}

const DOCUMENT_DATE: Slot = slot(
    "document_date",
    None,
    Target::Subject,
    ValueKind::Date,
    DATE,
    "document date",
    "The date the document was issued or written",
);
const PERIOD_START: Slot = slot(
    "period_start",
    None,
    Target::Subject,
    ValueKind::Date,
    DATE,
    "period start",
    "First day of the period this statement covers",
);
const PERIOD_END: Slot = slot(
    "period_end",
    None,
    Target::Subject,
    ValueKind::Date,
    DATE,
    "period end",
    "Last day of the period this statement covers",
);

pub const PAYMENT_PERIODS: &[(&str, &str)] = &[
    ("month", "Paid every month"),
    ("quarter", "Paid every quarter"),
    ("half_year", "Paid every six months"),
    ("year", "Paid every year"),
    ("once", "A one-off amount or a total"),
    ("none", "The document does not say how often it is paid"),
];
const INSURANCE_KINDS: &[(&str, &str)] = &[
    ("health", "Health or long-term care insurance"),
    ("car", "Motor vehicle insurance"),
    ("liability", "Personal liability insurance"),
    ("household", "Household contents or building insurance"),
    ("legal", "Legal expenses insurance"),
    ("life", "Life, pension or disability insurance"),
    ("travel", "Travel insurance"),
    ("other", "Another kind of insurance, or not stated"),
];

const fn identity_document(key: &'static str, label: &'static str, what: &'static str) -> DocType {
    DocType {
        key,
        family: "identity",
        label,
        what,
        tier: Tier::Eager,
        entities: &[],
        slots: &[],
        links: &[],
    }
}

macro_rules! identity_type {
    ($key:literal, $label:literal, $what:literal, $namespace:literal, $kind:literal) => {
        DocType {
            entities: &[EntitySpec {
                role: "document",
                kind: EntityKind::GovernmentId,
                label_slot: None,
                fallback_label: $label,
                identifiers: &[IdentifierSpec {
                    namespace: $namespace,
                    slot: "number",
                    scope_slot: None,
                }],
                constants: &[("government_id.kind", $kind)],
            }],
            slots: &[
                required(mrz(
                    slot(
                        "number",
                        Some("government_id.number"),
                        Target::Role("document"),
                        ValueKind::Identifier,
                        IDENT,
                        "document number",
                        "The number of this identity document",
                    ),
                    MrzField::DocumentNumber,
                )),
                mrz(
                    slot(
                        "expires",
                        Some("government_id.expires"),
                        Target::Role("document"),
                        ValueKind::Date,
                        DATE,
                        "expiry date",
                        "The date until which the document is valid",
                    ),
                    MrzField::ExpiryDate,
                ),
                slot(
                    "issued",
                    Some("government_id.issued"),
                    Target::Role("document"),
                    ValueKind::Date,
                    DATE,
                    "date of issue",
                    "The date the document was issued",
                ),
                slot(
                    "issuer",
                    Some("government_id.issuer"),
                    Target::Role("document"),
                    ValueKind::Text,
                    TEXT,
                    "issuing authority",
                    "The authority that issued the document",
                ),
                mrz(
                    slot(
                        "birth_date",
                        Some("person.birth_date"),
                        Target::Subject,
                        ValueKind::Date,
                        DATE,
                        "date of birth",
                        "The holder's date of birth",
                    ),
                    MrzField::BirthDate,
                ),
                mrz(
                    slot(
                        "nationality",
                        Some("person.nationality"),
                        Target::Subject,
                        ValueKind::Identifier,
                        &[],
                        "nationality",
                        "The holder's nationality code",
                    ),
                    MrzField::Nationality,
                ),
            ],
            links: &[Link {
                from: Target::Subject,
                property: "person.government_id",
                to: Target::Role("document"),
                validity: SlotValidity::Timeless,
            }],
            ..identity_document($key, $label, $what)
        }
    };
}

const fn lazy(
    key: &'static str,
    family: &'static str,
    label: &'static str,
    what: &'static str,
) -> DocType {
    DocType {
        key,
        family,
        label,
        what,
        tier: Tier::Lazy,
        entities: &[],
        slots: &[],
        links: &[],
    }
}

const fn org(role: &'static str, label_slot: &'static str, fallback: &'static str) -> EntitySpec {
    EntitySpec {
        role,
        kind: EntityKind::Organization,
        label_slot: Some(label_slot),
        fallback_label: fallback,
        identifiers: &[],
        constants: &[],
    }
}
const fn name_slot(
    key: &'static str,
    role: &'static str,
    label: &'static str,
    description: &'static str,
) -> Slot {
    slot(
        key,
        None,
        Target::Role(role),
        ValueKind::Name,
        ORG,
        label,
        description,
    )
}

pub const DOC_TYPES: &[DocType] = &[
    identity_type!(
        "passport",
        "Passport",
        "A passport booklet data page",
        "passport.number",
        "passport"
    ),
    identity_type!(
        "id_card",
        "ID card",
        "A national identity card (Personalausweis)",
        "id_card.number",
        "id_card"
    ),
    identity_type!(
        "residence_permit",
        "Residence permit",
        "A residence permit or visa document",
        "residence_permit.number",
        "residence_permit"
    ),
    DocType {
        key: "driving_licence",
        family: "identity",
        label: "Driving licence",
        what: "A driving licence card (Führerschein)",
        tier: Tier::Eager,
        entities: &[EntitySpec {
            role: "document",
            kind: EntityKind::GovernmentId,
            label_slot: None,
            fallback_label: "Driving licence",
            identifiers: &[IdentifierSpec {
                namespace: "driving_licence.number",
                slot: "number",
                scope_slot: None,
            }],
            constants: &[("government_id.kind", "driving_licence")],
        }],
        slots: &[
            required(slot(
                "number",
                Some("government_id.number"),
                Target::Role("document"),
                ValueKind::Identifier,
                IDENT,
                "licence number",
                "The driving licence number (field 5)",
            )),
            slot(
                "issued",
                Some("government_id.issued"),
                Target::Role("document"),
                ValueKind::Date,
                DATE,
                "date of issue",
                "The date the licence was issued (field 4a)",
            ),
            slot(
                "expires",
                Some("government_id.expires"),
                Target::Role("document"),
                ValueKind::Date,
                DATE,
                "expiry date",
                "The date until which the licence card is valid (field 4b)",
            ),
            slot(
                "licence_since",
                Some("person.driving_licence_date"),
                Target::Subject,
                ValueKind::Date,
                DATE,
                "first licence date",
                "The date the holder first obtained a car (category B) licence",
            ),
            slot(
                "birth_date",
                Some("person.birth_date"),
                Target::Subject,
                ValueKind::Date,
                DATE,
                "date of birth",
                "The holder's date of birth",
            ),
        ],
        links: &[Link {
            from: Target::Subject,
            property: "person.government_id",
            to: Target::Role("document"),
            validity: SlotValidity::Timeless,
        }],
    },
    DocType {
        key: "wage_tax_certificate",
        family: "tax",
        label: "Wage tax certificate",
        what: "An annual Lohnsteuerbescheinigung issued by an employer",
        tier: Tier::Eager,
        entities: &[org("employer", "employer", "Employer")],
        slots: &[
            required(slot(
                "tax_id",
                Some("person.tax_id"),
                Target::Subject,
                ValueKind::Identifier,
                &[C::TaxId],
                "tax identification number",
                "The employee's personal tax identification number (IdNr)",
            )),
            name_slot(
                "employer",
                "employer",
                "employer",
                "The name of the employer who issued the certificate",
            ),
            valid(
                slot(
                    "gross_wage",
                    Some("person.gross_income"),
                    Target::Subject,
                    ValueKind::Money(Some(Period::Year)),
                    MONEY,
                    "gross wage",
                    "Line 3: Bruttoarbeitslohn einschl. Sachbezüge for the certificate period",
                ),
                SlotValidity::DocumentPeriod,
            ),
            employee_money(
                "wage_tax",
                "person.wage_tax",
                Period::Year,
                "wage tax",
                "Line 4: Einbehaltene Lohnsteuer von 3.",
            ),
            employee_money(
                "solidarity_surcharge",
                "person.solidarity_surcharge",
                Period::Year,
                "solidarity surcharge",
                "Line 5: Einbehaltener Solidaritätszuschlag von 3.",
            ),
            employee_money(
                "church_tax",
                "person.church_tax",
                Period::Year,
                "church tax",
                "Line 6: Einbehaltene Kirchensteuer des Arbeitnehmers von 3. (line 7 is the spouse's, not this)",
            ),
            employee_money(
                "pension_contribution",
                "person.pension_contribution",
                Period::Year,
                "pension contribution",
                "Line 23 a: Arbeitnehmeranteil zur gesetzlichen Rentenversicherung (line 22 is the employer's)",
            ),
            employee_money(
                "health_contribution",
                "person.health_insurance_contribution",
                Period::Year,
                "health insurance contribution",
                "Line 25: Arbeitnehmerbeiträge zur gesetzlichen Krankenversicherung",
            ),
            employee_money(
                "care_contribution",
                "person.care_insurance_contribution",
                Period::Year,
                "care insurance contribution",
                "Line 26: Arbeitnehmerbeiträge zur sozialen Pflegeversicherung",
            ),
            employee_money(
                "unemployment_contribution",
                "person.unemployment_insurance_contribution",
                Period::Year,
                "unemployment insurance contribution",
                "Line 27: Arbeitnehmerbeiträge zur Arbeitslosenversicherung",
            ),
            PERIOD_START,
            PERIOD_END,
        ],
        links: &[Link {
            from: Target::Subject,
            property: "person.employer",
            to: Target::Role("employer"),
            validity: SlotValidity::DocumentPeriod,
        }],
    },
    DocType {
        key: "tax_assessment",
        family: "tax",
        label: "Tax assessment",
        what: "A Steuerbescheid or other assessment notice from a tax office",
        tier: Tier::Eager,
        entities: &[org("office", "tax_office", "Tax office")],
        slots: &[
            slot(
                "tax_id",
                Some("person.tax_id"),
                Target::Subject,
                ValueKind::Identifier,
                &[C::TaxId],
                "tax identification number",
                "The taxpayer's personal tax identification number (IdNr)",
            ),
            valid(
                slot(
                    "tax_number",
                    Some("person.tax_number"),
                    Target::Subject,
                    ValueKind::Identifier,
                    IDENT,
                    "tax number",
                    "The Steuernummer assigned by the tax office",
                ),
                SlotValidity::From("document_date"),
            ),
            name_slot(
                "tax_office",
                "office",
                "tax office",
                "The name of the tax office (Finanzamt) that issued the notice",
            ),
            slot(
                "tax_year",
                None,
                Target::Subject,
                ValueKind::Period,
                PERIOD,
                "tax year",
                "The calendar year assessed (Veranlagungszeitraum)",
            ),
            employee_money(
                "taxable_income",
                "person.taxable_income",
                Period::Year,
                "taxable income",
                "Zu versteuerndes Einkommen for the tax year",
            ),
            employee_money(
                "income_tax_assessed",
                "person.income_tax_assessed",
                Period::Year,
                "assessed income tax",
                "Festgesetzte Einkommensteuer for the tax year, not amounts already paid or still due",
            ),
            valid(
                slot(
                    "tax_balance",
                    Some("person.tax_balance"),
                    Target::Subject,
                    ValueKind::Balance,
                    MONEY,
                    "refund or back payment",
                    "The remaining amount of this assessment: Erstattung (refund) or Nachzahlung (payment due)",
                ),
                SlotValidity::DocumentPeriod,
            ),
            valid(
                slot(
                    "payment_due",
                    Some("person.tax_payment_due"),
                    Target::Subject,
                    ValueKind::Date,
                    DATE,
                    "payment due date",
                    "The date by which a back payment must be paid, if printed",
                ),
                SlotValidity::DocumentPeriod,
            ),
            DOCUMENT_DATE,
        ],
        links: &[Link {
            from: Target::Subject,
            property: "person.tax_office",
            to: Target::Role("office"),
            validity: SlotValidity::From("document_date"),
        }],
    },
    lazy(
        "tax_return",
        "tax",
        "Tax return",
        "A tax return (Steuererklärung) or its attachments",
    ),
    lazy(
        "donation_receipt",
        "tax",
        "Donation receipt",
        "A donation receipt (Zuwendungsbestätigung)",
    ),
    DocType {
        key: "payslip",
        family: "employment",
        label: "Payslip",
        what: "A monthly salary statement (Gehaltsabrechnung, Entgeltabrechnung)",
        tier: Tier::Eager,
        entities: &[
            org("employer", "employer", "Employer"),
            org("health_insurer", "health_insurer", "Health insurer"),
        ],
        slots: &[
            name_slot(
                "employer",
                "employer",
                "employer",
                "The name of the employer",
            ),
            required(employee_money(
                "gross",
                "person.gross_income",
                Period::Month,
                "gross pay",
                "Total gross pay for this month (Gesamtbrutto), not year-to-date totals",
            )),
            employee_money(
                "net",
                "person.net_income",
                Period::Month,
                "net pay",
                "Net pay for this month (Nettoverdienst), before other deductions such as advances",
            ),
            employee_money(
                "wage_tax",
                "person.wage_tax",
                Period::Month,
                "wage tax",
                "Lohnsteuer withheld from this month's pay, not the year-to-date total",
            ),
            employee_money(
                "solidarity_surcharge",
                "person.solidarity_surcharge",
                Period::Month,
                "solidarity surcharge",
                "Solidaritätszuschlag withheld this month",
            ),
            employee_money(
                "church_tax",
                "person.church_tax",
                Period::Month,
                "church tax",
                "Kirchensteuer withheld from the employee this month",
            ),
            valid(
                slot(
                    "tax_class",
                    Some("person.tax_class"),
                    Target::Subject,
                    ValueKind::Category(TAX_CLASSES),
                    &[],
                    "tax class",
                    "The wage tax class (Steuerklasse, StKl) applied this month",
                ),
                SlotValidity::DocumentPeriod,
            ),
            employee_money(
                "pension_contribution",
                "person.pension_contribution",
                Period::Month,
                "pension contribution",
                "Employee share of statutory pension insurance (RV-Beitrag AN) this month",
            ),
            employee_money(
                "health_contribution",
                "person.health_insurance_contribution",
                Period::Month,
                "health insurance contribution",
                "Employee share of statutory health insurance including the additional contribution (KV-Beitrag AN) this month",
            ),
            employee_money(
                "care_contribution",
                "person.care_insurance_contribution",
                Period::Month,
                "care insurance contribution",
                "Employee share of long-term care insurance (PV-Beitrag AN) this month",
            ),
            employee_money(
                "unemployment_contribution",
                "person.unemployment_insurance_contribution",
                Period::Month,
                "unemployment insurance contribution",
                "Employee share of unemployment insurance (AV-Beitrag AN) this month",
            ),
            slot(
                "pay_month",
                None,
                Target::Subject,
                ValueKind::Period,
                PERIOD,
                "pay month",
                "The month this payslip covers (Abrechnungsmonat)",
            ),
            slot(
                "tax_id",
                Some("person.tax_id"),
                Target::Subject,
                ValueKind::Identifier,
                &[C::TaxId],
                "tax identification number",
                "The employee's personal tax identification number",
            ),
            slot(
                "social_insurance_number",
                Some("person.social_insurance_number"),
                Target::Subject,
                ValueKind::Identifier,
                &[C::SocialInsuranceNumber],
                "social insurance number",
                "The employee's social insurance (pension insurance) number",
            ),
            valid(
                slot(
                    "employee_number",
                    Some("person.employee_number"),
                    Target::Subject,
                    ValueKind::Identifier,
                    IDENT,
                    "employee number",
                    "The employee or personnel number (Personalnummer)",
                ),
                SlotValidity::DocumentPeriod,
            ),
            name_slot(
                "health_insurer",
                "health_insurer",
                "health insurer",
                "The employee's statutory health insurance fund (Krankenkasse)",
            ),
            PERIOD_START,
            PERIOD_END,
        ],
        links: &[
            Link {
                from: Target::Subject,
                property: "person.employer",
                to: Target::Role("employer"),
                validity: SlotValidity::DocumentPeriod,
            },
            Link {
                from: Target::Subject,
                property: "person.health_insurer",
                to: Target::Role("health_insurer"),
                validity: SlotValidity::DocumentPeriod,
            },
        ],
    },
    DocType {
        key: "employment_contract",
        family: "employment",
        label: "Employment contract",
        what: "An employment contract (Arbeitsvertrag) or its amendment",
        tier: Tier::Eager,
        entities: &[org("employer", "employer", "Employer")],
        slots: &[
            required(name_slot(
                "employer",
                "employer",
                "employer",
                "The employing company",
            )),
            slot(
                "start",
                None,
                Target::Subject,
                ValueKind::Date,
                DATE,
                "start date",
                "The first day of employment",
            ),
            valid(
                slot(
                    "occupation",
                    Some("person.occupation"),
                    Target::Subject,
                    ValueKind::Text,
                    TEXT,
                    "job title",
                    "The employee's position or job title",
                ),
                SlotValidity::From("start"),
            ),
            DOCUMENT_DATE,
        ],
        links: &[Link {
            from: Target::Subject,
            property: "person.employer",
            to: Target::Role("employer"),
            validity: SlotValidity::From("start"),
        }],
    },
    lazy(
        "reference_letter",
        "employment",
        "Reference letter",
        "An employer reference (Arbeitszeugnis) or certificate of employment",
    ),
    DocType {
        key: "insurance_policy",
        family: "insurance",
        label: "Insurance policy",
        what: "An insurance policy document (Versicherungsschein) or policy change notice",
        tier: Tier::Eager,
        entities: &[
            org("insurer", "insurer", "Insurer"),
            EntitySpec {
                role: "contract",
                kind: EntityKind::InsuranceContract,
                label_slot: Some("insurer"),
                fallback_label: "Insurance",
                identifiers: &[IdentifierSpec {
                    namespace: "contract.number",
                    slot: "number",
                    scope_slot: Some("insurer"),
                }],
                constants: &[],
            },
        ],
        slots: &[
            required(name_slot(
                "insurer",
                "insurer",
                "insurer",
                "The insurance company that issued the policy",
            )),
            required(slot(
                "number",
                Some("contract.number"),
                Target::Role("contract"),
                ValueKind::Identifier,
                IDENT,
                "policy number",
                "The insurance policy number (Versicherungsscheinnummer)",
            )),
            slot(
                "kind",
                Some("insurance.kind"),
                Target::Role("contract"),
                ValueKind::Category(INSURANCE_KINDS),
                &[],
                "kind of insurance",
                "What the policy insures",
            ),
            valid(
                slot(
                    "premium",
                    Some("contract.premium"),
                    Target::Role("contract"),
                    ValueKind::Money(None),
                    MONEY,
                    "premium",
                    "The premium the policyholder pays, including insurance tax where printed",
                ),
                SlotValidity::From("start"),
            ),
            slot(
                "start",
                Some("contract.start_date"),
                Target::Role("contract"),
                ValueKind::Date,
                DATE,
                "start date",
                "The date the insurance cover begins",
            ),
            DOCUMENT_DATE,
        ],
        links: &[
            Link {
                from: Target::Role("contract"),
                property: "contract.provider",
                to: Target::Role("insurer"),
                validity: SlotValidity::Timeless,
            },
            Link {
                from: Target::Role("contract"),
                property: "contract.holder",
                to: Target::Subject,
                validity: SlotValidity::From("start"),
            },
        ],
    },
    DocType {
        key: "health_insurance_notice",
        family: "insurance",
        label: "Health insurance notice",
        what: "A membership, contribution or card letter from a health insurance fund",
        tier: Tier::Eager,
        entities: &[org("insurer", "insurer", "Health insurer")],
        slots: &[
            required(name_slot(
                "insurer",
                "insurer",
                "health insurer",
                "The health insurance fund that sent the letter",
            )),
            valid(
                slot(
                    "member_number",
                    Some("person.health_insurance_number"),
                    Target::Subject,
                    ValueKind::Identifier,
                    IDENT,
                    "health insurance number",
                    "The insured person's membership number (Versichertennummer)",
                ),
                SlotValidity::From("document_date"),
            ),
            DOCUMENT_DATE,
        ],
        links: &[Link {
            from: Target::Subject,
            property: "person.health_insurer",
            to: Target::Role("insurer"),
            validity: SlotValidity::From("document_date"),
        }],
    },
    lazy(
        "insurance_claim",
        "insurance",
        "Claim letter",
        "A letter about an insurance claim or benefit",
    ),
    DocType {
        key: "bank_account_document",
        family: "banking",
        label: "Bank account document",
        what: "A bank statement or account opening confirmation for the holder's own account",
        tier: Tier::Eager,
        entities: &[
            org("bank", "bank", "Bank"),
            EntitySpec {
                role: "account",
                kind: EntityKind::BankAccount,
                label_slot: Some("bank"),
                fallback_label: "Bank account",
                identifiers: &[IdentifierSpec {
                    namespace: "iban",
                    slot: "iban",
                    scope_slot: None,
                }],
                constants: &[],
            },
        ],
        slots: &[
            required(slot(
                "iban",
                Some("bank_account.iban"),
                Target::Role("account"),
                ValueKind::Identifier,
                &[C::Iban],
                "account IBAN",
                "The IBAN of the account holder's own account, not a payee's account",
            )),
            slot(
                "bic",
                Some("bank_account.bic"),
                Target::Role("account"),
                ValueKind::Identifier,
                &[C::Bic],
                "BIC",
                "The BIC of the account holder's bank",
            ),
            name_slot("bank", "bank", "bank", "The bank that keeps the account"),
        ],
        links: &[
            Link {
                from: Target::Subject,
                property: "person.bank_account",
                to: Target::Role("account"),
                validity: SlotValidity::Timeless,
            },
            Link {
                from: Target::Role("account"),
                property: "bank_account.bank",
                to: Target::Role("bank"),
                validity: SlotValidity::Timeless,
            },
        ],
    },
    lazy(
        "card_or_loan",
        "banking",
        "Card or loan letter",
        "A credit card, loan or other bank product letter",
    ),
    DocType {
        key: "vehicle_registration",
        family: "vehicle",
        label: "Vehicle registration",
        what: "A vehicle registration certificate (Zulassungsbescheinigung Teil I or II, Fahrzeugschein, Fahrzeugbrief)",
        tier: Tier::Eager,
        entities: &[EntitySpec {
            role: "vehicle",
            kind: EntityKind::Vehicle,
            label_slot: Some("make"),
            fallback_label: "Vehicle",
            identifiers: &[
                IdentifierSpec {
                    namespace: "vin",
                    slot: "vin",
                    scope_slot: None,
                },
                IdentifierSpec {
                    namespace: "vehicle.registration",
                    slot: "plate",
                    scope_slot: None,
                },
            ],
            constants: &[],
        }],
        slots: &[
            slot(
                "plate",
                Some("vehicle.registration"),
                Target::Role("vehicle"),
                ValueKind::Identifier,
                &[C::Plate],
                "registration plate",
                "The official registration number (amtliches Kennzeichen)",
            ),
            slot(
                "vin",
                Some("vehicle.vin"),
                Target::Role("vehicle"),
                ValueKind::Identifier,
                &[C::Vin],
                "vehicle identification number",
                "The vehicle identification number (FIN, field E)",
            ),
            slot(
                "make",
                Some("vehicle.make"),
                Target::Role("vehicle"),
                ValueKind::Text,
                TEXT,
                "make",
                "The manufacturer brand (field D.1)",
            ),
            slot(
                "model",
                Some("vehicle.model"),
                Target::Role("vehicle"),
                ValueKind::Text,
                TEXT,
                "model",
                "The model or trade name (field D.3)",
            ),
            slot(
                "first_registration",
                Some("vehicle.first_registration"),
                Target::Role("vehicle"),
                ValueKind::Date,
                DATE,
                "first registration",
                "The date of first registration (field B)",
            ),
            DOCUMENT_DATE,
        ],
        links: &[Link {
            from: Target::Subject,
            property: "person.owns",
            to: Target::Role("vehicle"),
            validity: SlotValidity::From("document_date"),
        }],
    },
    lazy(
        "vehicle_other",
        "vehicle",
        "Vehicle document",
        "A vehicle purchase, service or inspection document",
    ),
    DocType {
        key: "rental_contract",
        family: "housing",
        label: "Rental contract",
        what: "A residential rental contract (Mietvertrag) or rent change notice",
        tier: Tier::Eager,
        entities: &[
            EntitySpec {
                role: "home",
                kind: EntityKind::Address,
                label_slot: Some("street"),
                fallback_label: "Home",
                identifiers: &[IdentifierSpec {
                    namespace: "address",
                    slot: "street",
                    scope_slot: Some("postal_code"),
                }],
                constants: &[],
            },
            EntitySpec {
                role: "contract",
                kind: EntityKind::Contract,
                label_slot: None,
                fallback_label: "Rental contract",
                identifiers: &[],
                constants: &[],
            },
            org("landlord", "landlord", "Landlord"),
        ],
        slots: &[
            required(slot(
                "street",
                Some("address.street"),
                Target::Role("home"),
                ValueKind::Text,
                &[C::Street],
                "street and number",
                "Street and house number of the rented home",
            )),
            slot(
                "postal_code",
                Some("address.postal_code"),
                Target::Role("home"),
                ValueKind::Identifier,
                &[C::PostalCode],
                "postal code",
                "Postal code of the rented home",
            ),
            slot(
                "city",
                Some("address.city"),
                Target::Role("home"),
                ValueKind::Text,
                &[C::City],
                "city",
                "City of the rented home",
            ),
            name_slot(
                "landlord",
                "landlord",
                "landlord",
                "The landlord or property management company",
            ),
            valid(
                slot(
                    "rent",
                    Some("contract.payment"),
                    Target::Role("contract"),
                    ValueKind::Money(Some(Period::Month)),
                    MONEY,
                    "monthly rent",
                    "The total monthly rent including service charges (Gesamtmiete / Warmmiete)",
                ),
                SlotValidity::From("start"),
            ),
            slot(
                "start",
                Some("contract.start_date"),
                Target::Role("contract"),
                ValueKind::Date,
                DATE,
                "tenancy start",
                "The date the tenancy begins",
            ),
            DOCUMENT_DATE,
        ],
        links: &[
            Link {
                from: Target::Subject,
                property: "person.residence",
                to: Target::Role("home"),
                validity: SlotValidity::From("start"),
            },
            Link {
                from: Target::Role("contract"),
                property: "contract.holder",
                to: Target::Subject,
                validity: SlotValidity::From("start"),
            },
            Link {
                from: Target::Role("contract"),
                property: "contract.provider",
                to: Target::Role("landlord"),
                validity: SlotValidity::Timeless,
            },
        ],
    },
    DocType {
        key: "residence_registration",
        family: "housing",
        label: "Residence registration",
        what: "A residence registration certificate (Meldebescheinigung, Anmeldebestätigung)",
        tier: Tier::Eager,
        entities: &[EntitySpec {
            role: "home",
            kind: EntityKind::Address,
            label_slot: Some("street"),
            fallback_label: "Home",
            identifiers: &[IdentifierSpec {
                namespace: "address",
                slot: "street",
                scope_slot: Some("postal_code"),
            }],
            constants: &[],
        }],
        slots: &[
            required(slot(
                "street",
                Some("address.street"),
                Target::Role("home"),
                ValueKind::Text,
                &[C::Street],
                "street and number",
                "Street and house number of the registered residence",
            )),
            slot(
                "postal_code",
                Some("address.postal_code"),
                Target::Role("home"),
                ValueKind::Identifier,
                &[C::PostalCode],
                "postal code",
                "Postal code of the registered residence",
            ),
            slot(
                "city",
                Some("address.city"),
                Target::Role("home"),
                ValueKind::Text,
                &[C::City],
                "city",
                "City of the registered residence",
            ),
            slot(
                "move_in",
                None,
                Target::Subject,
                ValueKind::Date,
                DATE,
                "move-in date",
                "The date the person moved into this residence (Einzugsdatum)",
            ),
            DOCUMENT_DATE,
        ],
        links: &[Link {
            from: Target::Subject,
            property: "person.residence",
            to: Target::Role("home"),
            validity: SlotValidity::From("move_in"),
        }],
    },
    lazy(
        "utility_statement",
        "housing",
        "Utility statement",
        "A utility bill or service charge statement (Nebenkostenabrechnung)",
    ),
    DocType {
        key: "service_contract",
        family: "contract",
        label: "Service contract",
        what: "A contract confirmation for phone, internet, energy, gym or a similar service",
        tier: Tier::Eager,
        entities: &[
            org("provider", "provider", "Provider"),
            EntitySpec {
                role: "contract",
                kind: EntityKind::Contract,
                label_slot: Some("provider"),
                fallback_label: "Contract",
                identifiers: &[IdentifierSpec {
                    namespace: "contract.number",
                    slot: "number",
                    scope_slot: Some("provider"),
                }],
                constants: &[],
            },
        ],
        slots: &[
            required(name_slot(
                "provider",
                "provider",
                "provider",
                "The company providing the service",
            )),
            slot(
                "number",
                Some("contract.number"),
                Target::Role("contract"),
                ValueKind::Identifier,
                IDENT,
                "contract number",
                "The contract or customer number for this service",
            ),
            valid(
                slot(
                    "payment",
                    Some("contract.payment"),
                    Target::Role("contract"),
                    ValueKind::Money(None),
                    MONEY,
                    "recurring fee",
                    "The regular fee the customer pays for the service",
                ),
                SlotValidity::From("start"),
            ),
            slot(
                "start",
                Some("contract.start_date"),
                Target::Role("contract"),
                ValueKind::Date,
                DATE,
                "start date",
                "The date the contract begins",
            ),
            slot(
                "end",
                Some("contract.end_date"),
                Target::Role("contract"),
                ValueKind::Date,
                DATE,
                "end date",
                "The date the contract ends, if stated",
            ),
            DOCUMENT_DATE,
        ],
        links: &[
            Link {
                from: Target::Role("contract"),
                property: "contract.provider",
                to: Target::Role("provider"),
                validity: SlotValidity::Timeless,
            },
            Link {
                from: Target::Role("contract"),
                property: "contract.holder",
                to: Target::Subject,
                validity: SlotValidity::From("start"),
            },
        ],
    },
    lazy(
        "subscription",
        "contract",
        "Subscription",
        "A newspaper, streaming or other subscription",
    ),
    lazy(
        "medical_report",
        "health",
        "Medical document",
        "A medical report, finding or treatment plan",
    ),
    lazy(
        "prescription_or_note",
        "health",
        "Prescription or sick note",
        "A prescription or sick note (Arbeitsunfähigkeitsbescheinigung)",
    ),
    lazy("invoice", "invoice", "Invoice", "An invoice or bill"),
    lazy(
        "receipt",
        "invoice",
        "Receipt",
        "A receipt or proof of purchase",
    ),
    lazy(
        "letter",
        "correspondence",
        "Letter",
        "A personal or administrative letter",
    ),
    lazy("email", "correspondence", "Email", "An email message"),
    lazy(
        "advertising",
        "noise",
        "Advertising",
        "Advertising or a newsletter",
    ),
];

/// Document types of one family, in registry order.
pub fn family_types(family: &str) -> impl Iterator<Item = &'static DocType> + '_ {
    DOC_TYPES.iter().filter(move |t| t.family == family)
}
pub fn doc_type(key: &str) -> Option<&'static DocType> {
    DOC_TYPES.iter().find(|t| t.key == key)
}
pub fn family(key: &str) -> Option<&'static Family> {
    FAMILIES.iter().find(|f| f.key == key)
}
impl DocType {
    pub fn slot(&self, key: &str) -> Option<&Slot> {
        self.slots.iter().find(|s| s.key == key)
    }
    pub fn entity(&self, role: &str) -> Option<&EntitySpec> {
        self.entities.iter().find(|e| e.role == role)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn registry_is_internally_consistent_and_uses_the_vocabulary() {
        let vocabulary: BTreeSet<_> = crate::vocabulary().map(|p| p.key).collect();
        let standard = [
            "person.tax_id",
            "person.social_insurance_number",
            "person.birth_date",
        ];
        let known = |key: &str| vocabulary.contains(key) || standard.contains(&key);
        let mut keys = BTreeSet::new();
        for t in DOC_TYPES {
            assert!(keys.insert(t.key), "duplicate type {}", t.key);
            assert!(family(t.family).is_some(), "unknown family for {}", t.key);
            let mut slots = BTreeSet::new();
            for s in t.slots {
                assert!(slots.insert(s.key), "duplicate slot {}.{}", t.key, s.key);
                if let Some(p) = s.property {
                    assert!(known(p), "{}.{} uses undefined {p}", t.key, s.key);
                }
                if let Target::Role(r) = s.target {
                    assert!(
                        t.entity(r).is_some(),
                        "{}.{} targets unknown role {r}",
                        t.key,
                        s.key
                    );
                }
                if let SlotValidity::From(from) = s.validity {
                    assert!(
                        t.slot(from).is_some(),
                        "{}.{} validity uses unknown slot",
                        t.key,
                        s.key
                    );
                }
                let categorical = matches!(s.value, ValueKind::Category(_));
                assert!(
                    categorical || s.mrz.is_some() || !s.accepts.is_empty(),
                    "{}.{} offers no candidates",
                    t.key,
                    s.key
                );
            }
            for e in t.entities {
                for i in e.identifiers {
                    assert!(t.slot(i.slot).is_some(), "{}: identifier slot", t.key);
                    if let Some(scope) = i.scope_slot {
                        assert!(t.slot(scope).is_some(), "{}: scope slot", t.key);
                    }
                }
                if let Some(label) = e.label_slot {
                    assert!(t.slot(label).is_some(), "{}: label slot", t.key);
                }
                for (property, _) in e.constants {
                    assert!(known(property));
                }
            }
            for l in t.links {
                assert!(known(l.property), "{} link {}", t.key, l.property);
                for end in [l.from, l.to] {
                    if let Target::Role(r) = end {
                        assert!(t.entity(r).is_some(), "{} link role {r}", t.key);
                    }
                }
                if let SlotValidity::From(from) = l.validity {
                    assert!(t.slot(from).is_some());
                }
            }
            assert_eq!(
                t.tier == Tier::Eager,
                !t.slots.is_empty(),
                "{}: only eager types declare slots",
                t.key
            );
        }
        for f in FAMILIES {
            if !["other", "noise"].contains(&f.key) {
                assert!(
                    family_types(f.key).next().is_some(),
                    "family {} has no types",
                    f.key
                );
            }
        }
    }

    #[test]
    fn tax_documents_list_the_tax_checklist() {
        let keys =
            |t: &str| -> Vec<&str> { doc_type(t).unwrap().slots.iter().map(|s| s.key).collect() };
        let payslip = keys("payslip");
        for k in [
            "gross",
            "net",
            "wage_tax",
            "solidarity_surcharge",
            "church_tax",
            "tax_class",
            "pension_contribution",
            "health_contribution",
            "care_contribution",
            "unemployment_contribution",
            "pay_month",
        ] {
            assert!(payslip.contains(&k), "payslip lacks {k}");
        }
        let certificate = keys("wage_tax_certificate");
        for k in [
            "gross_wage",
            "wage_tax",
            "solidarity_surcharge",
            "church_tax",
            "pension_contribution",
            "health_contribution",
            "care_contribution",
            "unemployment_contribution",
        ] {
            assert!(certificate.contains(&k), "certificate lacks {k}");
        }
        let assessment = keys("tax_assessment");
        for k in [
            "tax_year",
            "taxable_income",
            "income_tax_assessed",
            "tax_balance",
            "payment_due",
        ] {
            assert!(assessment.contains(&k), "assessment lacks {k}");
        }
        let balance = doc_type("tax_assessment")
            .unwrap()
            .slot("tax_balance")
            .unwrap();
        assert_eq!(balance.value, ValueKind::Balance);
        assert_eq!(
            doc_type("payslip").unwrap().slot("wage_tax").unwrap().value,
            ValueKind::Money(Some(Period::Month))
        );
        assert_eq!(
            doc_type("wage_tax_certificate")
                .unwrap()
                .slot("wage_tax")
                .unwrap()
                .value,
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
}
