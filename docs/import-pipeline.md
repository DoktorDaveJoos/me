# Importing documents that people can trust

Research and implementation decision, updated 28 September 2026.

The current [TypeSafe, recovery and spending design](import-reliability.md)
details the per-stage checkpoint implementation and provider boundaries.

The product promise is to preserve the original, explain what was read, and show
what remains uncertain. It is not a claim of perfect OCR or perfect AI interpretation.
A plausible answer without the right person, period and source is a failed import
outcome, even when every character in the answer occurs somewhere in the file.

## Deep-first reading

Implemented 27–28 September 2026; design in
[the deep-first extraction spec](superpowers/specs/2026-09-27-deep-first-extraction-design.md).
It replaces the tiered pipeline's candidate selection and gap fill. Every
document, automatic or started manually, is read in full by one pipeline
(`apps/desktop/src/read_import.rs`); the pure steps are in
`crates/me-agent/src/read_assembly.rs`. The registry type decides which values
may enter the profile, not whether a document is read.

1. **Intake.** Window drops, file picking and folder dumps create envelopes.
   Identical bytes are stored once and a repeat links the existing source. A
   folder dump first shows one summary (files, size, unreadable formats, hidden
   files) and queues behind interactive drops.
2. **Normalize.** Unchanged local parsing and OCR, cut into canonical segments of
   at most 1,600 bytes.
3. **Classify.** One TypeSafe request per document: family, type, subject,
   legibility and mixed sources. A family below 0.5 becomes `other`; an uncertain
   type keeps only its family. A valid machine-readable zone decides the subject.
   The result is cached by content, registry, model and identity anchors, and
   saved as the document's profile on every read.
4. **Read.** The grounded reader (`codex_pipeline`) reads the whole document in
   3,200-byte parts with one segment of overlap. Its instructions carry the
   family/type guide and the type's checklist (slot key, label, description);
   each fact may carry a `slot` from that checklist or `none`. Each part gets a
   TypeSafe legibility judgment and an omission/attribution check, and at most one
   focused re-read when that check or local grounding flags it. Parts are
   checkpointed under the pipeline version and the guide, so a document classified
   again with another guide is read again.
5. **Ground.** Exact quote in its segment, value in the quote, subject and context
   in the source. A fact that fails is dropped and counted by its rejection code;
   one that only lacks its owner stays for verification.
6. **Omission sweep.** The local scanners run over the whole text. Checksum-valid
   identifiers (IBAN, tax ID, pension insurance number, MRZ) and amounts or dates
   with a printed label that no fact quotes trigger one focused audit request with
   the segments that hold them (at most 12,000 bytes). Whatever stays uncovered is
   stored as **not interpreted** with its label, printed value and location. An
   amount that starts a table cell takes the row's label cell on its left: text
   with more letters than digits, no `Label:` colon of its own and at most 256
   bytes. Joined EUR/Ct amounts on the Lohnsteuerbescheinigung, long numbered
   form rows and SAP rows with a letter wage-type code (`M010`) are therefore
   labeled and audited.
7. **Type locally.** Code locates each fact and types its value for the tagged
   slot: complete dates, month periods, years only after a printed period keyword
   (`Veranlagungszeitraum 2025`), checksums, tax class, and amounts with the
   currency rule (the document's single printed currency, else EUR for payslips,
   wage tax certificates and tax assessments, else none). A tag whose value cannot
   hold the slot is removed; the fact stays. A printed category that is not a code
   (a tariff name) keeps its tag for TypeSafe. Machine-readable-zone fields are
   filled by position, never from a tag.
8. **Verify.** One TypeSafe request per 20 facts, with the letterhead (first 12
   lines, at most 2,000 bytes). Every fact gets two failure Nouls (invented, off
   target) and an owner Choice (the anchors, up to 24 people named in the document,
   an organization, unclear). Amounts get a period Choice (this document's period,
   cumulative, another period, not periodic). A value counts as an amount when it
   prints a currency marker, exactly two decimals or a joined EUR/Ct pair. A whole
   number counts only when it is tagged for a money slot, so a tax ID or a
   personnel number is not asked about a period. Tagged facts get a mapping Noul and,
   where their slot needs it, a refund/payment, payment-period or category Choice.
   One correction Noul per document. When several facts claim one slot, one Choice
   among them decides. A fact's confidence is its weakest head.
9. **Profile values and persistence.** Machine-readable-zone fields, then every
   verified mapping, then the pay-month fallback (below). Resolve (policy
   `import-graph-v2`) turns them into assertions; every located fact becomes a
   `document_fact` row, and a `read_summary` row keeps counts and rejection codes.

**Verification bands.** A fired failure Noul (> 0.7) or a confidence below 0.6
rejects the assumption: the grounded fact stays on its document, marked
uncertain and detached from the profile. Below the vault's check threshold
(starts at 0.8, moves between 0.7 and 0.9 from the user's Quick-check outcomes) a
profile value is accepted and waits in **Quick checks**; a document fact gets an
**uncertain** marker and no question. Otherwise it is accepted silently and
marked unreviewed. A mapping also requires the subject as owner for the person's
own slots, this document's period for monthly or yearly amounts, a judged
direction for a refund or back payment, and no cumulative period for an open
premium or payment. Question texts and thresholds are in
`crates/me-agent/src/typesafe_questions.rs`; requests pin `jev-1.13.0`.

**Read store and Resolve.** Migration 16 adds `document_fact` (label, printed
value, location, owner, period, slot, linked assertion and state `verified`,
`uncertain`, `unverified` or `uninterpreted`) and `read_summary`. Money conflicts
compare only values of the same payment period, so monthly payslip values never
conflict with an annual certificate. A document judged a correction retracts
earlier automatic single-valued values of the same subject, property, employer
and exact period from other sources. A re-read replaces the source's document
facts and withdraws its support from earlier automatic values it no longer finds;
a value no other source supports is retracted. User decisions are never
overridden.

**Re-reading.** Migration 16 queues documents finished by the tiered reader for
a full read behind new imports (priority 0). The queue respects automatic
analysis, allowances and pauses. Cached classification is reused; reading and
verification run again.

**Allowances.** Every paid request is reserved durably before it is sent. Each
file has a lifetime allowance of 12 OpenAI calls, 64 TypeSafe requests and
180,000 reported tokens, extendable by 12/24/180,000. Files of a folder import
also share the import's allowance of 200 OpenAI calls, extendable by 200. Once
an import is used up, its remaining files stop before any paid step (before
Classify, before the reader and before the audit) with the typed reason
**Import allowance reached** (`BatchBudget`); a file at its own limit shows
**Analysis allowance reached**. Imports shows one notice per exhausted import
(“Allow more OpenAI calls for this import”); the stopped file offers “Allow more
OpenAI calls for this import and resume”. Extending an import re-queues only the
files it stopped, as queued while automatic analysis is on and as ready to start
otherwise. A file that runs out mid-document keeps its finished parts.

**Imports line.** A finished read shows its counts, for example “9 values read ·
4 in profile · 1 check · 3 not interpreted”. A file stopped at an allowance without a stored read
shows **Not read yet** or, once its extraction step had progress, **Partly
read**, with the reason. It is never shown as done. A stopped re-read keeps its
earlier read and its “Paused at …” line.

**Read from this document.** The document detail groups what was read into **In
your profile** (linked to an accepted profile value), **Other details** and **Not
interpreted**. Each row shows the printed label, the value in Geist Mono, its
location (“Page 1 · OCR”) and period, and “Waiting in Quick checks” or “Uncertain
reading” where they apply. The earlier “Found in this document” list stays below
it for older proposals.

**Failures.** A reader failure stops the file with a typed reason; usage-limit
and sign-in failures pause the queue. A failed verification stores every located
fact as unverified (owner unknown, never linked) and keeps the source's existing
profile values; the run stays resumable. A stopped verification stores nothing.

**Search.** **Look inside unread documents** asks one existence Noul per unread
document, several documents per request, and reads up to three matches first.

**Review, identity and reduce.** Quick checks, the identity setup, household and
merge proposals and the idle-time reduce judgments (spelling-equivalent
conflicts, organizations sharing a leading name word) are unchanged from the
tiered pipeline. No numeric confidence is displayed.

**Extraction self-check.** In builds with development tools (ME Dev), Settings →
Development → **Copy extraction self-check** copies one line per read document: family and type, values read, in
profile, checks, not interpreted, rejection codes and policy. It contains counts
and codes only, never titles, labels or values
([development.md](development.md#extraction-self-check)).

**Synthetic corpus and live evaluation.** `crates/me-core/fixtures/documents`
holds eleven invented documents in real layouts: DATEV payslips as OCR rows and
as a split-column text layer with appended OCR rows, SAP and Personio payslips,
a Lohnsteuerbescheinigung with EUR/Ct columns, a Steuerbescheid with a refund
and one with a back payment, an insurance letter, a bank statement, an invoice
and a passport with a machine-readable zone. `expectations.json` lists 191
expected facts with slot, period and owner. `crates/me-core/tests/corpus.rs`
checks that every value the scanners count as worth reading is expected, every
quote is verbatim and every expected profile value type-checks.
`crates/me-agent/tests/live_eval.rs` runs the real reader and TypeSafe over the
corpus and prints, per family, classification, slot recall and precision, fact
recall, owner and period accuracy, false completions, checks, values not
interpreted, and reader and TypeSafe requests per document. It is opt-in and
spends ChatGPT usage and TypeSafe requests:

```sh
ME_CODEX_TEST_HOME=<signed-in ME. Codex home> \
  ./scripts/cargo test -p me-agent --test live_eval -- --ignored --nocapture
```

### Known limits

- The omission audit runs only when scanner-visible values are uncovered. A
  missing required checklist slot alone does not trigger one. It sends at most
  12,000 bytes of the segments holding uncovered values; values beyond that stay
  not interpreted.
- The scanners still do not label every layout, so some misses are never
  audited. In a split-column text layer, the cells below a column's first have
  their label in another column block, not in their own row. Invoice line items
  have a quantity or unit price as the left cell, not a label. An amount after a
  value cell (a date, an IBAN) or after a label cell longer than 256 bytes is
  also missed.
- Reader checkpoints are namespaced by guide; a new classification with another
  guide pays for a new read.
- Pay-month fallback: when no fact fills `pay_month` or `tax_year`, the one
  period of the matching granularity printed in the context quotes of verified
  amounts for this document's period is used. Two different periods, or none,
  leave the slot empty.
- A failed verification stores unverified facts and keeps the source's earlier
  profile values until a resume succeeds.
- Corrections supersede only through an employer role. A corrected tax
  assessment never supersedes the earlier one; the difference stays a Quick
  check.
- `document_fact` and `read_summary` rows have no single-source purge path yet;
  only the development wipe removes them.
- Virtual filing uses the Classify family at its 0.5 floor.
- A whole number with no currency marker, decimals or money-slot tag (a free
  `4.200` on a tax assessment) is not asked the period question, so its
  document fact has no period. A reader tag for a money slot still asks it, also
  when the reader wrongly tagged an identifier; the mapping Noul judges that tag.
- The queue is ordered by priority and import order, not by type.
- Every automatic import pays for a full read, including documents whose type
  fills no profile values. A one-page document needs one reader request, one
  more per flagged part and one for the omission audit when values stay
  uncovered; on TypeSafe, one Classify request, two per part, one verification
  request per 20 facts and one per contested slot.

### Verification — 28 September 2026

Run on this Mac from the repository root, with the shared build cache:

- `./scripts/cargo fmt --all -- --check`: clean.
- `./scripts/check-design-system`: “Design system OK: 55 Rust files, 34
  consistent icons.” `python3 scripts/test-design-system.py`: 5 tests OK.
- `./scripts/cargo clippy --workspace --all-targets --all-features -- -D
  warnings`: clean (only the existing future-incompatibility note for `block` and
  `proc-macro-error2`).
- `./scripts/cargo test --workspace`: 407 passed, 0 failed, 11 ignored. Among
  them me-core 179 unit tests (1 ignored) with the new year-typing test, the
  corpus (2), me-agent 79 unit tests (4 ignored), the live evaluation's offline
  scorer check (1; the live evaluation itself is ignored), and me-app 51.
  `./scripts/cargo test -p me-agent --no-run` builds every me-agent test target.
- The row-label and amount-flag fix came later the same day. After it, `test
  --workspace` gave 410 passed, 0 failed, 11 ignored: me-core 181 unit tests,
  with the row-label and omission-sweep tests, and me-agent 80 unit tests, with
  the amount-flag test. The corpus stayed green without new expectations,
  because its quotes already covered every newly labeled value. `fmt` and
  `clippy` were clean.
- Synthetic gallery states inspected during the implementation (27–28
  September), window-only, at 1120×780 and 800×600: the “Read from this document”
  groups, the Imports read line with Not read yet and Partly read, and the
  exhausted-import notice; at 1120×820 and 800×600, the self-check button beside
  Wipe data, with the copied text and the notice. No UI changed for this
  documentation step, and these states were not inspected again.

Not verified:

- Live accuracy on the user's real documents. The user checks it in ME Dev.app;
  the self-check reports counts without contents.
- The live evaluation baseline: `live_eval` has not been run, so no recall,
  precision or cost figures exist yet. Eleven synthetic documents would not be a
  population accuracy measurement either, and thresholds are starting values,
  not calibrated accuracy.
- OS file drops, Linux rendering and runtime, screen readers, and the dark
  appearance of the new views.
- The expanded Not read yet and Partly read rows, and the `budget` and `complete`
  gallery modes.

## Findings from primary sources

| Finding | Consequence for ME. | Source |
| --- | --- | --- |
| Reading order, table structure and provenance belong in the document representation. Text alone loses relationships. | Keep page/section identities and original segments; preserve labels alongside values. Plan geometry and table cells as the next extractor upgrade. | [Docling document model](https://docling-project.github.io/docling/concepts/docling_document/) |
| OCR confidence and field-extraction confidence measure different things. One cannot stand in for the other; varied layouts need representative evaluation and review. | Evaluate transcription, semantics and completeness separately. Do not display model-generated confidence percentages as calibrated accuracy. | [Microsoft: interpreting accuracy and confidence](https://learn.microsoft.com/en-us/azure/ai-services/document-intelligence/concept/accuracy-confidence?view=doc-intel-4.0.0) |
| Apple Vision exposes recognized text candidates and confidence. | The existing on-device OCR is a useful baseline. A future quality gate should retain candidate confidence and boxes, rather than discard them after text conversion. | [Apple VNRecognizedText](https://developer.apple.com/documentation/vision/vnrecognizedtext) |
| Email is a multipart, encoded format, not a plain text file. | Decode MIME, character encodings, envelope fields and attachments with a maintained parser. Preserve attachment provenance. | [RFC 2045](https://www.rfc-editor.org/rfc/rfc2045), [mail-parser](https://github.com/stalwartlabs/mail-parser) |
| Asynchronous scheduling does not make CPU-heavy or blocking work nonblocking. Unbounded blocking pools still need explicit concurrency limits and cooperative cancellation. | Run parsers and provider processes off the GPUI thread, bound active documents, release the vault mutex before OCR/model work, and cancel the actual child process. | [Tokio spawn_blocking](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html) |

Docling is a credible candidate for a layout-aware local extraction adapter. Its
structured model addresses a real gap in the current text-focused extractors. It
is not an automatic choice for the desktop bundle: evaluate distribution size,
startup cost, memory, offline model assets and difficult German correspondence
before introducing another runtime. The present change keeps the native
PDFKit/Vision and Poppler/Tesseract implementations.

## User flow implemented now

1. Drop one or many files anywhere in the window, including Settings or over a
   dialog. File-picker and file-paste use the same confirmation. While locked or
   signing in, file paths wait in memory until the app can show confirmation.
2. The dialog lists names, paths, formats, sizes and eligibility. Deselect the
   wrong files or cancel. Metadata inspection does not read document contents or
   send anything to a provider. Files are revalidated when read at import time.
3. Confirm to save selected originals in the encrypted vault. Unsupported
   analysis formats are explicitly marked as original-only. 1Password exports
   retain their separate credential flow and cannot join a mixed document batch.
4. Imports in the sidebar shows each saved file, its current stage, real page or
   section counts, elapsed time, failure and retry controls, or review outcome.
   The overall percentage measures completed work across five stage bars, never
   estimated elapsed time. Each planned section moves through the stages in order.
5. What was read is usable at once and marked unreviewed; doubtful profile
   values wait in Quick checks. Confirming the file import never confirms what
   was read, and few values read is not proof of completeness.

Search’s explicit Add a file / file-paste flow retains its existing form-matching
behavior after confirmation. It reserves these files for the form reader so the
automatic fact-extraction workers cannot race the form workflow. Window drops
and Add files on Imports use the new general import pipeline.

The Imports page uses the shared design system: Geist, the existing outline icon
family, semantic colors, spacing tokens and the standard 8 px radius.

## Pipeline and stage boundaries

| Stage | Work | Evidence of progress |
| --- | --- | --- |
| Save original | Read a bounded regular file; encrypt and durably store it. | Per-file saving/queued state; success only after persistence. |
| Normalization | Decode text/Office/MIME or locally recognize PDF/image text. Preserve source sections and attachment boundaries. | Real OCR page counts where the parser provides them. |
| Interpretation | TypeSafe classifies the document (family, type, subject, legibility, mixed sources) and judges each part's legibility. | Completed section count and cached typed decisions. |
| Context | Apply the local reading guide of the document's family and type, and its profile checklist. | Explicitly says local guidance; no web search is claimed. |
| Extraction | Extract documented values with their labels, person, period, proposed profile slot and exact source references. | Section counts; the immutable original is retained. |
| Verification | Local grounding; per-part TypeSafe omission/attribution checks with at most one focused re-read; the document's omission sweep with at most one audit; local typing; TypeSafe verification of each fact's meaning, owner, period and slot. | Completed checks; doubtful profile values become Quick checks; values nobody interpreted stay listed. |
| Review | Store profile values through Resolve, document facts and not-interpreted values. | The Imports read line and “Read from this document”. |

For an insurance letter, sender, recipient, insured person and policyholder must
not collapse into one person. Policy number, issue date, coverage dates, amount,
payee and requested action have different meanings. Preserve negation and
conditions: a rejected claim is not an approved benefit; a generic policy limit
is not proof that this person is covered. Preserve relative deadline wording;
do not invent a calendar date without the required reference event.

For email, decode the claimed sender, recipients, subject, date and body. A From
header does not authenticate identity. Quoted history and signatures are distinct
contexts. `.eml` now uses a Rust MIME parser and reads supported attachments
locally, including nested messages (bounded depth). Attachment sections retain
their own labels and page locations. Damaged encodings or unsupported attachments
stop analysis visibly rather than silently disappear. Originals remain available.
HTML is converted locally without fetching remote images or following links.
Outlook `.msg` is not supported; export as EML or PDF.

## Where external research belongs

Research can clarify a term or a form convention. It cannot recover a smudged
member number, establish private insurance coverage, replace missing pages, or
prove that an amount belongs to the right person. Re-OCR, inspect the page or ask
a targeted question for those cases.

A future research adapter should receive a narrow structured request: document
family, language/jurisdiction, public issuer/template identifier, unfamiliar term,
and the reason context is needed. It should not receive the private document,
name, address, account/member/policy number, medical contents or free-form model
search query. Prefer a versioned local reference; otherwise use approved primary
sources. Record URL, retrieval date, relevant scope and the interpretation it
supports. Treat retrieved material as untrusted content, with no ability to run
actions or modify the original evidence.

Allow at most a small bounded number of lookups, cache public context independently
of personal data, and display the actual lookup status. If essential context
cannot be established, finish with an explicit unresolved question rather than a
guess. External research remains an architecture extension; this implementation
makes no network lookups for document interpretation. The context stage is real
local guidance, not a simulated research animation.

## Rust execution and persistence

ME. already has GPUI's background executor and synchronous, cancellable parser
and provider subprocess adapters. Adding Tokio solely to create parallelism would
add an unnecessary second scheduler. This implementation runs at most two active
document workers on the existing executor. Each document preserves the order of
its own stages while different documents can progress independently. The limit
also bounds large decrypted originals and concurrent OCR/provider processes.

Vault access uses short serialized operations for document intake, claims,
checkpoints and final writes. OCR and model calls release that lock. Original
intake is deliberately serialized because the current vault owns a single
SQLCipher connection. A future throughput optimization should split bounded file
reads/encryption from the transaction using an explicit intake API, not share the
connection unsafely. No performance claim is made without release measurements.

Migrations 9 and 10 store typed stages, individual step counters, intermediate
results and durable request allowances in SQLCipher. A run ID rejects late
progress writes from a previous attempt. Existing evaluation state remains the
source of truth for queued/running/manual/done/failed. On restart, interrupted jobs
remain failed and require explicit resume; the previous stage remains diagnostic
context, not a claim that a worker is still running. Saved steps can be reused.
Quota, rate-limit and authentication failures also pause the queue persistently. Stop, vault lock and connection failure signal
cancellation; locking clears visible filenames, progress and pending selections.
Disabling automatic analysis stops automatic workers and prevents new claims.

The UI receives typed events rather than inferring stages from translated status
strings. Elapsed time continues to update during a slow model call; it is not a
progress estimate. The pipeline cache version changes when interpretation guidance
changes, avoiding reuse of older cached semantics for a new analysis.

## Quality gates and evaluation plan

Existing guarantees: bounded parsing, exact source grounding, no silent text
truncation, typed omission sweep and conditional audit, checkpoint validation,
TypeSafe verification of every fact with Quick checks for doubtful profile
values, and no automatic user confirmation. These are necessary, but an exact
quote alone does not establish correct semantics or full recall.

The next extractor milestone should introduce a structured intermediate document:
page/section, ordered blocks, table row/cell coordinates, OCR alternatives and
confidence, language, decoder provenance, warnings, and attachment relationships.
A quality gate can then route ambiguous regions to a second local pass or to a
consented visual model. Neither that richer layout model nor a visual-model
fallback is implemented in this change.

A first synthetic corpus of eleven layouts and its opt-in live evaluation exist
(see [Deep-first reading](#deep-first-reading)); no baseline has been recorded
yet. Still to build: a versioned, consented or synthetic corpus spanning plain text,
German posted letters, health-insurance notices, policies/claims, invoices, native
and scanned PDFs, mixed PDFs, rotated phone photos, multi-column tables, email
alternatives, nested attachments, mixed languages and multi-person bundles. Hold
out issuer/template families, not just pages from the same template.

Score separately: character/digit accuracy, document/party/period assignment,
field precision/recall, evidence location, complete-document coverage, false
completion rate, and manual corrections per document. Track p50/p95 duration,
peak memory, cancellation latency and restart recovery as engineering metrics.
Do not choose confidence thresholds from model self-reports. Calibrate routing
against the labeled corpus and require no silent loss or invented critical
identifiers in the release regression set. A small clean fixture set is not a
population accuracy measurement.

Current automated additions cover MIME header/charset/body decoding, HTML without
script execution, supported attachment evidence, unsupported attachment failure,
independent queue claims, restart recovery, stale progress rejection, and metadata
preflight. The broader existing suite covers OCR formats, grounding, omissions,
provider failure/cancellation, large document checkpoints and migration behavior.
Live model accuracy across these correspondence families remains to be measured.


## Verification — 18 September 2026

- Formatting, shared design-system guard and its five regression checks pass.
- Workspace Clippy with all targets and warnings denied passes.
- Workspace tests: 112 passed. Five opt-in real-CLI/model tests remain ignored.
- The optimized macOS release bundle builds and is locally signed.
- Synthetic native UI inspection covered the progress page at normal size and
  confirmation at 800×600, including long filenames, scrolling, deselection and
  committing only the two selected files. The displayed active workers in the
  gallery are synthetic; these screenshots are not live model accuracy evidence.
- Root and occluding dialog drop handlers use the pinned GPUI API. An actual OS
  drag gesture over every screen and Linux runtime behavior were not exercised.
- No personal documents were used for tests. Broad document-family accuracy,
  live external research, and a visual-model fallback remain unverified or future
  work as described above.
