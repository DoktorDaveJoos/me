# Recoverable imports and TypeSafe decisions

Implementation and primary-source research, 18 September 2026.

## What changed and why

Imports queried all collection items sharing a document source. Confirmed atomic
facts therefore appeared as separate imports. The query now selects documents
only. No facts or originals are deleted.

The observed failed extraction spent minutes on large sections, split work after
failure, and repeated model passes. An interruption could discard already paid
intermediate results. The new pipeline uses small, predetermined sections with
neighbor overlap, durable results per step, and no application retry loop or
recursive splitting. Progress counts finished work, not time spent waiting.

## TypeSafe research and boundaries

TypeSafe evaluates typed questions over supplied state. Choice selects a known
category; Noul expresses the probability of a yes/no answer. Those primitives fit
routing and review decisions. They do not generate arbitrary document facts or
transcribe images. ME. batches related questions into a request using `jev-latest`
and validates every answer's type, option, range and probability distribution.
See the [API reference](https://docs.typesafe.ai/api) and
[primitives](https://docs.typesafe.ai/primitives).

A high score is not proof of correctness. The initial 0.7 document-family and 0.2
review thresholds are conservative engineering choices, not measured accuracy
claims. Ambiguous categories use general guidance; unreadable text stops before
extraction; uncertain semantics trigger one focused review. A representative
German correspondence/payroll/insurance corpus is still needed to calibrate these
choices. See [TypeSafe confidence](https://docs.typesafe.ai/confidence) and the
[document-quality plan](import-pipeline.md#quality-gates-and-evaluation-plan).

| Stage | Implemented work | Progress and recovery |
| --- | --- | --- |
| Normalization | Local decoders and OCR; preserve original and source references. | Actual pages when known. Email attachment totals are not presented as the total for the entire email. Indexed text is reused. |
| Interpretation | TypeSafe family, legibility, tabular-layout and mixed-source decisions. | One completed decision per section; validated decisions are cached. |
| Context | Apply local payroll/correspondence guidance and cautious layout instructions. | Local completed sections. No web research or generation call. |
| Extraction | OpenAI generates documented facts with exact source quotations, person and period. | Completed sections. Raw structured results are saved before later checks. |
| Verification | Deterministic source grounding plus TypeSafe omission and attribution checks. If indicated, allow one OpenAI audit. Ground its results again. | Complete only after the checks finish. Unsupported candidates remain questions. |

Virtual filing reuses the saved TypeSafe categories locally. Consistent,
sufficiently confident classifications map to Employment, Insurance,
Correspondence or Invoices; missing/mixed decisions map to Unsorted. This removes
the previous additional OpenAI organization loop. Search, chat and explicit form
assistance are separate features; their decisions have not all been migrated to
TypeSafe in this change.

## Progress, persistence and recovery

Each file starts collapsed, showing its filename, status and overall bar.
**Show details** expands the five stage bars, provider usage, diagnostics and
pause/resume controls. A failed stage and error category remain visible when
collapsed. Expansion survives progress updates and navigation until the vault
locks. The overall value is the mean
of completed fractions across the five stages, labeled **Completed work**. It is
not a time estimate; stages have different costs. An unknown total uses an
activity marker and Working label. A remote call never acquires a fake percentage
because time passed. Stage counters advance only on completion.

Extraction favors retaining legible, source-backed candidates even when ownership,
terminology or context is uncertain. Unknown meanings keep their printed document
label; they are not guessed into a profile field. An empty `subject_quote` explicitly
requests ownership review. Grounding checks the source, value and any supplied
context first, then stores a question such as **Is this your tax ID?**. Only an
explicit user confirmation makes it personal. Missing ownership alone does not
trigger a paid audit; TypeSafe still checks for omissions and incorrect associations.
The request schema and cache namespace stay compatible so existing paid steps are
reused. New or unfinished extraction calls use the more inclusive guidance;
completed imports are not silently reanalyzed.

Migration 10 adds step progress, typed provider/error codes, intermediate results,
request reservations, allowances and a queue pause inside SQLCipher. A run ID
rejects late progress, usage and completion writes. Cache identity includes
immutable source content, stage inputs and pipeline version; attempt IDs are
excluded. Completed grounded v4 sections can be reused. Raw intermediate JSON is
untrusted working data, not a confirmed fact.

Pause, cancellation, crash or restart leaves the file stopped with its previous
stage and saved work. **Resume saved steps** is explicit. Resuming a failed file
keeps its allowance and saved steps. Explicit reanalysis of a successfully
completed document clears its analysis cache, but retains usage. Originals and
confirmed facts remain intact. Provider quota, rate-limit and authentication
failures pause the queue persistently; they do not repeatedly cycle files.

Errors identify provider, failure class and stage. Provider response bodies and
private payloads are not logged or displayed. Structured errors distinguish quota,
rate limit, sign-in, timeout, invalid output, connection, source and vault errors.
The UI uses a bounded explanation and a recovery action.

## Spending controls and their limits

Default **per-file lifetime allowance**: 12 OpenAI model calls, 24 TypeSafe HTTP
requests and 180,000 reported input plus output tokens. Reservations are stored
before dispatch and count even when a timeout or crash makes the outcome unknown.
A retry, restart or reanalysis does not reset them. Usage events are absolute per
request and deduplicated. Missing usage remains visibly unreported rather than
being treated as zero spend. Increasing the allowance requires the separately
labeled action on that file; it adds another 12/24 calls and 180,000 tokens.

At most two document workers run. Each section normally uses two TypeSafe calls
and one OpenAI call, with at most one additional extraction audit. New sections
contain up to 3,200 bytes of canonical source text with adjacent overlap. Large
files can exhaust the default allowance and require explicit continuation. Output
that reaches the 96-fact per-pass limit stops visibly instead of being silently
accepted as complete. Library callers without vault accounting also have a
96-request per-attempt guard; desktop imports use the stricter durable allowance.

The TypeSafe transport performs no retries. Codex uses an explicitly named OpenAI
provider fixed to the ChatGPT backend, with HTTP and stream retries set to zero;
the built-in provider ID cannot be overridden in CLI 0.146.0. A quota preflight
runs before inference. No credit redemption, model fallback or paid OpenAI API
fallback is used. See the [Codex configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference)
and [provider implementation](https://github.com/openai/codex/blob/main/codex-rs/model-provider-info/src/lib.rs).

These are request and reported-token controls, **not a guaranteed currency cap**.
A request may be charged before cancellation, token reporting can arrive late or
never arrive, and the provider owns any server-side work or authentication
recovery. Already-started parallel work can finish after a queue pause. The token
threshold can be exceeded by an in-flight request; the ledger prevents subsequent
requests. Set account-side spending limits where available as an additional bound.

## Private configuration

Copy `.env.example` to `.env`, set your own `TYPESAFE_API_KEY`, and run
`chmod 600 .env`. Development builds read the project file at runtime; nothing is
embedded in the binary. Alternatively set the environment variable, or point
`ME_TYPESAFE_ENV_FILE` at a private file. Desktop launches do not normally inherit
shell variables. Distributed builds need that explicit private file location or
environment configuration; the developer's compile-time source directory is not
a portable installation setting. Symlinked, nonregular or group/world-readable
key files are refused. Git ignores `.env`; the bundler does not copy it.

Only use credentials issued to the deployment owner. The confirmation dialog
explains that extracted content goes to TypeSafe for decisions and OpenAI for
extraction. Credentials from 1Password remain excluded. No private document data
is sent to a web search.

## Verification

Behavioral tests cover atomic-fact filtering, schema migration, encrypted partial
results, restart, explicit resume, stale workers, exhausted allowances, deduplicated
usage, queue pause, cancellation, no automatic retries, partial-step reuse across
new attempt IDs, exact evidence and invalid provider output. The installed CLI
handshake/login-start test uses a temporary home and makes no inference request.
One live TypeSafe request used a tiny synthetic invoice (521 input / 112 output
tokens). No personal document was used and no live OpenAI extraction was run for
this change. Synthetic UI checks cover progress, quota and allowance states.

This establishes control-flow and recovery behavior. It does not establish
perfect semantic interpretation, full recall, calibrated confidence, or Linux
runtime behavior. OCR geometry, cross-section context and difficult tables remain
accuracy work; human review is still required.
