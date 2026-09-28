# Deep-first extraction with TypeSafe verification — design

Design agreed 27 September 2026. Status: implemented 27–28 September 2026; see
[import-pipeline.md](../../import-pipeline.md#deep-first-reading) for verified
behavior and limits.
Supersedes the extraction part of
[the tiered import pipeline design](2026-09-27-import-pipeline-design.md)
(steps 4 and 5 of its architecture). Intake, normalization, classification,
Resolve, checkpoints and allowances stay.

## Goal

Extraction quality is the product. A person must be able to trust that ME. read
what a document says, attributed it to the right person and period, and asks only
when it is genuinely unsure. Cost is secondary to recall and correctness, but it
stays bounded, visible and never degrades a result silently.

## Problem

German payslips imported with the tiered pipeline finished without errors and
yielded only the employee number. Reproduced with a synthetic DATEV-style page:

1. The money scanner requires a currency marker next to every amount
   (`candidates.rs`, `scan_money`). Payroll tables print `EUR` once in a column
   header or not at all. Gesamt-Brutto, Lohnsteuer and Netto became untyped
   label/value candidates; the only money candidate was the bank-transfer line.
2. Money slots accept only money candidates (`doc_types.rs`, `MONEY`). With no
   options the slot question is skipped silently (`typesafe_questions.rs`,
   `extract_questions`). Identifier slots also accept label/value candidates,
   which is why the employee number survived.
3. Gap fill cannot recover: it reads only the first 3,200 bytes, re-runs the same
   scanner on the quote, offers a label/value candidate the money slot rejects,
   drops rejected facts, and ignores reader failures (`graph_import.rs`).
4. The registry has no slots for Lohnsteuer, Solidaritätszuschlag, Kirchensteuer,
   Steuerklasse or social contributions; the Lohnsteuerbescheinigung has only
   gross wage and tax ID. Anything outside a slot is unreachable by
   `apply_document_graph`.
5. `Januar 2026` and `01/2026` are not complete dates, so payslip periods are
   lost and validity becomes unknown.
6. TypeSafe sees only the first 12 lines plus candidate lines; lazy types are never
   extracted during import.
7. Tests used a fixture with `€` on every amount and hand-written candidates; the
   scanner never ran on realistic payroll text.

The earlier grounded deep reader (`codex_document.rs`) already had open-ended
extraction and a detailed payroll guide, but it was used only for explicit
**Run AI**, stored facts as proposals that each needed approval, and never fed
the personal graph.

## Decisions

| Topic | Decision |
| --- | --- |
| Primary extractor | The grounded deep reader reads every document in full, automatic or manual. One pipeline. |
| Registry role | Slots are the profile checklist (what must be looked for and may enter the graph), not a whitelist. Every other grounded fact is kept as a document fact. |
| Mapping | The reader proposes a slot tag per fact; TypeSafe verifies it. |
| Local scanners | Type and validate the reader's quotes and sweep for omissions. They never filter what is read. |
| Verification | TypeSafe validates every reader assumption: meaning, owner, period and profile mapping. |
| Review | Confident assumptions are accepted silently; uncertain ones become one Quick check; confidently wrong assumptions are dropped while the grounded text stays on the document. |
| Budget | Never degrade silently. Out of allowance means **Not read yet** or **Partly read**, never done. |
| Existing imports | Documents read by `import-graph-v1` are re-read automatically behind new imports. |

## Pipeline

Per document, in order. Each step checkpoints its output as today.

1. **Normalize.** Unchanged local OCR and parsing.
2. **Classify.** Unchanged TypeSafe request (family, type, subject, legibility,
   mixed). The type selects the reading guide and checklist and orders the queue
   (eager types first). It no longer decides whether a document is read.
3. **Read.** `codex_pipeline` over the whole document in its existing 3,200-byte
   parts with one-segment overlap. Instructions: the existing safety and
   open-ended extraction rules, the family/type guide, and the type's checklist
   (slot key, label and description). The output schema gains `slot`: one of the
   checklist keys or `none`. The per-section interpret request keeps legibility,
   table and mixed-source judgments; the document kind comes from Classify.
4. **Ground.** Unchanged `grounding.rs`: exact quote in segment, value in quote,
   subject and context in source. Failures are dropped and counted.
5. **Type locally.** `find_candidates` runs on each fact's quote and context:
   checksums (IBAN, tax ID, SV number, MRZ), ISO dates, month and year periods,
   amounts with the currency rule below, tax class. A slot tag whose value does
   not type-check (for example `gross` on a non-amount) is removed; the fact stays.
6. **Verify.** See below.
7. **Omission sweep.** Scanners run over the full text. Checksum-valid
   identifiers, labeled amounts and labeled dates not covered by any fact quote,
   plus missing required checklist slots, trigger at most one focused audit
   request per document naming those lines (existing audit mechanism). Audit facts
   pass steps 4–6. Values still uncovered are stored as **not interpreted**.
8. **Persist.** Verified slot facts become graph assertions through Resolve
   (validity, conflicts, quick checks, evidence). All other grounded facts become
   document facts. Not-interpreted values are stored with label, raw value and
   location.

Removed: local-candidate slot selection as the extractor, gap fill, and "lazy
means never read". `graph_pipeline::extract` and `verify_gap_fill` are replaced.

## Verification and review

One batched TypeSafe request per roughly 20 grounded facts. State: the fact, its
quote, its source line with one line of context, the document type and the
identity anchors. Questions per fact:

| Reader assumption | Question | Primitive |
| --- | --- | --- |
| Meaning | Is the value invented? Is it off target (for example year-to-date taken as monthly, employer share as employee share)? | One Noul per failure mode, true means wrong (existing SDE cascade) |
| Owner | You, a household member, another named party, the issuing organization itself, or unclear | Choice |
| Period (money only) | This document's period, cumulative/year-to-date, another stated period, not periodic | Choice |
| Mapping (slot-tagged only) | Is this the `<slot>` of this document? | Noul |
| Competing mappings | Several facts tagged with one slot: which one? | Choice over the reader's facts plus `none` |
| Correction | Is this document a correction or cancellation of an earlier one (`Korrektur`, `Stornierung`, `Nachberechnung`)? | Noul, once per document |

Confidence of a fact is the weakest head: `1 − max(failure Nouls)`, owner
confidence, period confidence and mapping confidence where asked.

| Band | Outcome | The user sees |
| --- | --- | --- |
| Confidence ≥ threshold | Accepted automatically with evidence, `unreviewed` | Nothing; the value appears |
| Floor ≤ confidence < threshold | Accepted provisionally | Profile values: one Quick check naming the doubtful assumption, e.g. "Is 1.032,58 € your Lohnsteuer for January 2026?". Document facts: an **uncertain** marker on the document, no question |
| Confidence < floor, or any failure Noul > 0.7 | The assumption (mapping, owner) is rejected | Nothing; the grounded fact stays on the document, detached from the profile |

The threshold is the existing adaptive check threshold (starts at 0.8, moves
between 0.7 and 0.9 from the user's own Quick-check outcomes). The floor is the
existing `SLOT_FLOOR` (0.6). Scores are never shown as percentages. The
existing `subject_unknown` review question is replaced by the owner Choice and
asked only in the Quick-check band. Question texts and thresholds stay in
`typesafe_questions.rs`.

## Registry, guides and values

### Guides

Guides live in `me-agent` beside the existing payroll and correspondence guides,
one per family with optional per-type refinements. They rely on printed labels
first; official line numbers are hints because numbering changes between form
years. A guide supplies meanings, never values.

| Family / type | Guide essentials |
| --- | --- |
| Employment: payslip | Existing DATEV guide, plus: Steuer-/SV-Brutto are not Gesamtbrutto; Jahreswerte are cumulative; Nachberechnung months are separate periods; employee vs. employer contributions; Netto-Verdienst vs. Auszahlungsbetrag. |
| Tax: Lohnsteuerbescheinigung | Table below. Amounts sit in split `EUR` and `Ct` columns; join the two cells of one row into one value and quote both. `Korrektur` replaces an earlier certificate for the same employer and period; `Stornierung` cancels it. |
| Tax: Steuerbescheid | festgesetzt vs. anzurechnen vs. verbleibend; Erstattung vs. Nachzahlung; Vorauszahlungen for later years are not this year's tax; Veranlagungszeitraum is the tax year. |
| Identity | Printed fields vs. MRZ; issue vs. expiry date; issuing authority. |
| Banking | Balance vs. transaction; debit vs. credit; whose IBAN. |
| Invoice | Net, VAT and gross; due date; the payee's IBAN belongs to the payee. |
| Contract, insurance, housing | Start, end and notice period without computing dates; premium period; insured person vs. policyholder vs. payee (existing correspondence rules). |
| Other | General rules only. |

Lohnsteuerbescheinigung 2026, verified against the BMF sample printout
(BMF-Schreiben of 29 August 2025, IV C 5 – S 2533/00123/007/007):

| Line | Printed label (abridged) | Slot |
| --- | --- | --- |
| 1 | Bescheinigungszeitraum (vom – bis) | `period_start`, `period_end` |
| 3 | Bruttoarbeitslohn einschl. Sachbezüge | `gross_wage` |
| 4 | Einbehaltene Lohnsteuer von 3. | `wage_tax` |
| 5 | Einbehaltener Solidaritätszuschlag von 3. | `solidarity_surcharge` |
| 6 | Einbehaltene Kirchensteuer des Arbeitnehmers von 3. | `church_tax` |
| 7 | Kirchensteuer des Ehegatten/Lebenspartners | none (spouse) |
| 22a/b | Arbeitgeberanteil/-zuschuss Renten-/Versorgungseinrichtungen | none (employer) |
| 23a | Arbeitnehmeranteil zur gesetzlichen Rentenversicherung | `pension_contribution` |
| 24a–c | Steuerfreie Arbeitgeberzuschüsse KV/PV | none (employer) |
| 25 | Arbeitnehmerbeiträge zur gesetzlichen Krankenversicherung | `health_contribution` |
| 26 | Arbeitnehmerbeiträge zur sozialen Pflegeversicherung | `care_contribution` |
| 27 | Arbeitnehmerbeiträge zur Arbeitslosenversicherung | `unemployment_contribution` |
| 28 | unbesetzt from 2026 (previously Vorsorgepauschale private KV/PV) | none |
| Header | Identifikationsnummer, Personalnummer, Steuerklasse/Faktor, Kirchensteuermerkmale | `tax_id`; others are document facts |

Every other line is still read and kept as a document fact.

### Profile values

New slots are marked `+`. All money values are employee values.

- **Payslip** (monthly): `gross`, `net`, `+wage_tax`, `+solidarity_surcharge`,
  `+church_tax`, `+tax_class`, `+pension_contribution`, `+health_contribution`
  (including Zusatzbeitrag), `+care_contribution`, `+unemployment_contribution`,
  `+pay_month`, plus the existing `tax_id`, `social_insurance_number`,
  `employee_number`, `employer`, `health_insurer`, `period_start`, `period_end`.
- **Lohnsteuerbescheinigung** (annual): `gross_wage`, `+wage_tax`,
  `+solidarity_surcharge`, `+church_tax`, `+pension_contribution`,
  `+health_contribution`, `+care_contribution`, `+unemployment_contribution`,
  period from line 1, `tax_id`, `employer`.
- **Steuerbescheid**: `+tax_year`, `+taxable_income`, `+income_tax_assessed`,
  `+tax_balance`, `+payment_due`, plus the existing `tax_id`, `tax_number`,
  `tax_office`, `document_date`.
- **Other types**: slots unchanged; more can follow from observed outcomes.

New vocabulary properties: `person.wage_tax`, `person.solidarity_surcharge`,
`person.church_tax`, `person.pension_contribution`,
`person.health_insurance_contribution`, `person.care_insurance_contribution`,
`person.unemployment_insurance_contribution`, `person.taxable_income`,
`person.income_tax_assessed`, `person.tax_balance` (money) and
`person.tax_class` (category I–VI, printed as `1`–`6` or Roman numerals).
`tax_balance` is signed in code: an Erstattung is negative, a Nachzahlung
positive; the direction comes from a TypeSafe Choice, never from the model's
arithmetic. `REGISTRY_VERSION` becomes `doc-types-v2`.

### Currency

1. If the document prints exactly one currency anywhere (`EUR`, `€`, `Euro`, an
   ISO code), bare amounts carry it.
2. Otherwise the type default applies: EUR for payslip,
   Lohnsteuerbescheinigung and Steuerbescheid.
3. Otherwise the amount stays without a currency. It cannot become a profile
   money value and remains a document fact. A currency is never guessed.

Bare amounts are typed only from a fact quote or a labeled line: two decimal
places with `,` or `.` as decimal separator and consistent grouping, or a
joined `EUR`/`Ct` cell pair.

### Periods

Month tokens (`Januar 2026`, `Jan. 2026`, `01/2026`, `01.2026`, `2026-01`) become
a month; year tokens in period context (`Veranlagungszeitraum 2025`, `für 2025`)
become a year. Code computes first and last days. Payslip validity falls back to
`pay_month` when explicit from/to dates are missing; Steuerbescheid validity is
its tax year.

### Resolve changes

- Money conflicts compare only values with the same payment period. A monthly
  payslip value never conflicts with an annual certificate value.
- A document judged a correction supersedes earlier automatic values of the same
  property, employer and period instead of conflicting with them.
- A re-read of a source replaces that source's earlier automatic values. Values
  the re-read no longer finds are retracted unless the user decided on them. User
  decisions are never overridden. Graph policy becomes `import-graph-v2`.

## Persistence

- Profile values: existing assertions, evidence, `assertion_review` and quick
  checks.
- Document facts: rows of a new `document_fact` table with printed label, raw
  value, locator, owner, period, profile slot, linked assertion and state
  (`verified`, `uncertain`, `unverified`, `uninterpreted`). A separate table
  avoids rebuilding `observation`, whose state CHECK constraint SQLite cannot
  alter in place. They replace per-fact `ai_proposal` rows for new reads.
  Existing pending proposals and questions stay until the user handles them.
- Not interpreted: `document_fact` rows with state `uninterpreted`.
- A `read_summary` row per source records counts and rejection codes for the
  Imports line and the self-check.
- Schema migration 16 adds both tables and re-queues documents read by
  `import-graph-v1`.

## User interface

Uses the shared design system only: existing components, semantic colors,
spacing tokens, the 8 px radius, Geist for labels and Geist Mono for values.

- **Document detail**: the approve/dismiss "Found in this document" list becomes
  a read-only **Read from this document** view with three groups: **In your
  profile** (verified mapped values with period), **Other details** (document
  facts) and **Not interpreted**. Each row shows label, value, period and a jump
  to the source quote. This document's Quick checks appear inline.
- **Review**: Quick checks only. The bulk "Approve these details as mine" flow is
  retired for new reads.
- **Imports**: a one-line result per file, for example "34 values read · 12 in
  profile · 1 check · 2 not interpreted". New states **Not read yet** (with the
  reason and the existing Extend action) and **Partly read**.

## Re-reading existing imports

On upgrade, sources whose graph policy is `import-graph-v1` are queued for a
re-read behind new imports. The queue respects automatic-analysis settings,
allowances and pauses. Cached classification is reused; extraction versions
change, so reading and verification run again.

## Errors and budget

| Failure | Result |
| --- | --- |
| Reader failure (connection, usage limit, authentication) | **Not read yet** with a typed reason. Usage-limit and authentication failures pause the queue (existing). No failure is swallowed. |
| Allowance reached mid-document | Finished sections stay checkpointed; the document is **Partly read**, resumable, and never finalized early. |
| TypeSafe verification failure | Facts are stored as document facts marked **not verified yet**. Nothing is promoted to the profile or asked. Resume retries verification only. |
| Grounding failure | Dropped deterministically, counted in diagnostics. |

A typical one- or two-page payslip needs about 2–3 reader requests and about 3
TypeSafe requests. Existing limits stay: 12 reader requests per file (about
38 KB of text) and 200 per import batch, both extendable. Diagnostics record
counts and codes only, never labels, values or file names.

## Testing and evaluation

**Synthetic corpus** under `me-core` test fixtures, no personal data, copying
real layouts: DATEV Brutto/Netto as OCR rows and as a split-column PDF text
layer; an SAP/ADP-style payslip; a Personio-style payslip with `€` per value; a
Lohnsteuerbescheinigung in the BMF 2026 layout with split EUR/Ct columns; a
Steuerbescheid with Erstattung and one with Nachzahlung; an insurance letter; a
bank statement; an invoice; a passport with MRZ. Each fixture lists expected
facts (label, value, slot, period, owner).

**Deterministic tests** (always run):

- Regression: a DATEV-like payslip yields gross, Lohnsteuer and net. Written
  first; it fails on the current code.
- Scanners: bare amounts, the currency rule, EUR/Ct joining, month and year
  tokens, tax class, typing of quotes.
- No silent loss: every checksum identifier, labeled amount and labeled date is
  covered by a fact or listed as not interpreted.
- Verification bands with scripted TypeSafe answers: accept, Quick check,
  reject; weakest-head rule; competing mappings; correction supersession.
- Persistence: slot facts become assertions with periods; twelve monthly
  payslips form a timeline without conflicts; monthly and annual values do not
  conflict; document facts and not-interpreted values are stored; a re-read
  replaces automatic values and keeps user decisions; exhausted allowance leaves
  **Not read yet** or **Partly read**.

**Live evaluation** (opt-in, ignored by default): the real reader and TypeSafe
over the corpus. Reports per family field recall and precision, owner and
period accuracy, false-completion rate, checks per document and requests per
document. The baseline is recorded in `docs/import-pipeline.md`; thresholds are
not claimed as calibrated.

**Privacy-safe self-check**: a development command prints, per document, counts
only (values read, in profile, checks, not interpreted, rejection codes) so the
user's real documents can be diagnosed without exposing their contents.

**Visual**: synthetic gallery states for the document groups and Imports
states, inspected at normal and minimum window size.

**Gates**: formatting, Clippy, `./scripts/check-design-system`,
`python3 scripts/test-design-system.py` and the workspace tests.

## Out of scope and known limits

- Layout-aware geometry for split-column PDF text layers. The OCR row pass and
  the reader's column guidance cover it for now.
- Cross-checks in code between monthly and annual values (for example twelve
  payslips against the certificate).
- Guides for further jurisdictions; other countries use the general rules.
- Live accuracy on the user's real documents is verified by the user in
  ME Dev.app, not claimed by this design.

## Sources

- [BMF: Muster für den Ausdruck der elektronischen Lohnsteuerbescheinigung für 2026](https://www.bundesfinanzministerium.de/Content/DE/Downloads/BMF_Schreiben/Steuerarten/Lohnsteuer/2025-08-29-ausdruck-elektr-lstbesch-2026.html)
- [lohn-info.de: Lohnsteuerbescheinigung](https://www.lohn-info.de/lohnsteuerbescheinigung.html)
