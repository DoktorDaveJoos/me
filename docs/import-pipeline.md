# Importing documents that people can trust

Research and implementation decision, updated 18 September 2026.

The current [TypeSafe, recovery and spending design](import-reliability.md)
details the per-stage checkpoint implementation and provider boundaries.

The product promise is to preserve the original, explain what was read, and show
what remains uncertain. It is not a claim of perfect OCR or perfect AI interpretation.
A plausible answer without the right person, period and source is a failed import
outcome, even when every character in the answer occurs somewhere in the file.

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
5. Suggestions remain subject to the existing Review flow. Confirming file import
   never confirms inferred facts. No suggestions is not proof of completeness.

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
| Interpretation | TypeSafe evaluates document family, readability, table layout and mixed sources. | Completed section count and cached typed decisions. |
| Context | Apply local document guidance. Today this includes payslip distinctions and correspondence/insurance/email reading rules. | Explicitly says local guidance; no web search is claimed. |
| Extraction | Extract documented values with their labels, person, period and exact source references. | Section counts; the immutable original is retained. |
| Verification | Local grounding and TypeSafe omission/attribution checks; at most one focused OpenAI audit when indicated. | Completed checks; unresolved candidates remain questions. |
| Review | Persist supported suggestions and unresolved questions separately. | Counts and links to the existing document/review UI. |

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
truncation, typed omission check and conditional audit, checkpoint validation, separate uncertain
questions, and no automatic fact confirmation. These are necessary, but an exact
quote alone does not establish correct semantics or full recall.

The next extractor milestone should introduce a structured intermediate document:
page/section, ordered blocks, table row/cell coordinates, OCR alternatives and
confidence, language, decoder provenance, warnings, and attachment relationships.
A quality gate can then route ambiguous regions to a second local pass or to a
consented visual model. Neither that richer layout model nor a visual-model
fallback is implemented in this change.

Build a versioned, consented or synthetic evaluation corpus spanning plain text,
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
