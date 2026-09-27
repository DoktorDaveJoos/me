# Personal data import pipeline — design

Design agreed 27 September 2026. Status: approved for planning, not implemented.

## Goal

Let a person give ME. everything they have — thousands of contracts, passports,
tax mail, payslips and photos of paper — and derive a personal knowledge graph
from it cheaply. The same pipeline must later accept a continuous stream from a
scanner or mail forwarding without redesign.

This design extends the existing import pipeline
([import-pipeline.md](../../import-pipeline.md),
[import-reliability.md](../../import-reliability.md)) and the schema-14 personal
domain ([personal-domain.md](../../personal-domain.md)). Local OCR, encrypted
originals, grounding, observations, checkpoints and allowances are reused.

## Decisions

| Topic | Decision |
| --- | --- |
| Autonomy | Derived assertions are accepted automatically with `review_state = unreviewed`. Review happens on conflict, on low confidence, or at first use. This replaces the rule that import never confirms inferred facts. |
| Dump shape | Mostly loose PDFs and images in nested folders. Folder walk is the primary bulk source. ZIP and mailbox unpacking are out of scope. |
| Extraction depth | Tiered and lazy. Every file is normalized, classified and indexed. Only high-value document types are extracted eagerly; the rest are extracted on demand. |
| Graph reach | The user and declared household members are full `Person` entities. Everyone else is a lightweight party or `Organization`. |
| Identity anchors | An initial "Who are you?" setup declares name, former names, birth date and household before the dump. |
| Architecture | Envelope intake plus a durable staged job queue in `me-core` (approach 1 of 3). |
| Confidence | Assertions carry a confidence and its source. Below a threshold they become `check_suggested` and appear in Quick checks. Scores are never displayed as percentages. |
| Setup UI | Conversational, one question per screen, with the onboarding fingerprint drawing as the user answers. |
| Dump progress UI | A growing constellation (Knowledge map) with an honest progress bar and a one-line text ticker. |

## Architecture

```
Sources (produce envelopes only)          me-core durable stage queue
┌──────────────┐                     ┌────────────────────────────────────────┐
│ Drop / picker│──┐                  │ 1 Intake     hash, dedup, encrypt       │
│ Folder walk  │──┼─► Envelope ─────►│ 2 Normalize  local OCR/parse → segments │
│ (Scanner)    │──┤                  │ 3 Classify   TypeSafe: type, subject,   │
│ (Mail fwd)   │──┘                  │              currency, tier             │
└──────────────┘                     │ 4 Extract    local candidates →          │──► observations
                                     │              TypeSafe slot choice →      │
                                     │              OpenAI only on gaps         │
                                     │ 5 Resolve    observations → entities +   │──► entities / assertions
                                     │              assertions                  │
                                     └────────────────────────────────────────┘
          Lazy trigger (query or task needs a document) ──► re-enter at 4
```

### Units

**Envelope (`me-core`, new).** One intake event: `source_kind` (drop, folder,
scanner, mail), origin metadata (relative path, received time, sender for mail),
and one or more original files. A folder walk produces one envelope per file; a
mail produces one envelope with body and attachments; a scanner batch produces
one envelope with its pages. Envelopes link to existing `source` rows. Sources
know nothing about AI or stages.

**Stage (`me-core`, new trait).** `key`, `version`, and a run function that is
pure over saved inputs and injected providers. Output is checkpointed in
`stage_run`, keyed by `(content_hash, stage, version)`. Identical content never
runs a stage twice, across envelopes. Bumping one stage's version reruns only
that stage and later ones.

**Queue (`me-core`).** Replaces the fixed five-step `ImportStage` with per-envelope
stage state and priority: interactive drop > bulk dump > lazy backfill. Keeps run
IDs, stale-write rejection, durable allowances, pause on quota/rate-limit/auth,
and explicit resume.

**Providers (`me-agent`).** The existing TypeSafe client and Codex extractor
behind the `Decisions` and extractor traits. Stages receive providers by
injection so tests use fakes.

**Sources (`me-app`).** Window drop, file picker and folder walk. Walking,
hashing and reads run on the GPUI background executor, never the UI thread.

Unchanged: encrypted originals, OCR backends (PDFKit/Vision, Poppler/Tesseract),
grounding, observation provenance, replication triggers.

## Stage 1–2: Intake and Normalize

- Intake computes the SHA-256 content fingerprint (already stored on `source`).
  An identical fingerprint links the new envelope to the existing source and
  skips all later stages.
- Folder walk: recursive, does not follow symlinks, skips hidden and system files,
  preflights by extension and magic bytes, and streams envelopes into the queue
  while walking continues.
- Normalize is the existing local parsing and OCR. No change in behavior.

## Stage 3: Classify

Runs once per document, not per section. Input is the text of the first and last
page, capped at about 3 KB, plus the anchor names. One TypeSafe request asks:

1. **Family** (choice): identity, tax, insurance, employment, banking, housing,
   vehicle, health, contract, invoice/receipt, correspondence, noise/marketing,
   other.
2. **Type within the family** (choice), from the document type registry, for
   example identity → passport, ID card, driving licence, residence permit; tax →
   Lohnsteuerbescheinigung, Steuerbescheid, Steuererklärung, Spendenquittung.
3. **Subject** (choice): which anchor the document is about, or none.
4. **Current or superseded** (probability), for example an expired passport.

A family score below threshold classifies the document as `other` (lazy). The
result is stored as `source.document_type`. It also replaces the per-section
family decision for virtual filing.

### Document type registry

Versioned data in `me-core`, not code paths. Each type declares:

- `key`, `family`, `tier` (`eager` or `lazy`)
- `slots`: vocabulary property key, value type, candidate generators, required flag
- the entities it yields and how they link (see Resolve)
- optional local reading guidance (existing payroll/insurance guidance moves here)

Eager families at launch: identity, tax, insurance, employment/payslips, banking,
vehicle, housing/property, contracts. Adding a document type adds a registry
entry, not a pipeline change.

## Stage 4: Extract

Runs eagerly for `eager` types and on demand for `lazy` types. Cheapest first:

1. **Local candidate generators** over normalized text: MRZ (ICAO 9303, with check
   digits), IBAN (mod-97), German Steuer-ID (check digit), German and ISO dates,
   money amounts with currency, label/value pairs from the OCR line layout, and
   names matched against anchors.
2. **TypeSafe slot selection.** One request for all slots of the document. Per
   slot, a choice among the candidates plus "none". The chosen option's
   probability becomes the assertion confidence (`confidence_source = typesafe`).
   Values validated by a checksum get confidence 1.0 (`confidence_source = checksum`).
3. **OpenAI gap fill** only when a required slot has no candidate or its
   confidence is below threshold. One small call scoped to the missing slots,
   grounded exactly as today (`confidence_source = openai_grounded`).

Every extracted value becomes an immutable observation with its source locator,
as in schema 14.

**On-demand trigger.** When search, chat or a task needs facts from a lazy
document, it enqueues that document at Extract with interactive priority.

## Stage 5: Resolve

Turns observations into entities and assertions.

1. **Entity creation per type.** The registry declares yielded entities: passport
   or ID → `GovernmentId` linked to the subject person; insurance policy →
   `InsuranceContract` plus insurer `Organization`; payslip → employer
   `Organization`, `person.employer` and income; registration → `Vehicle`; bank
   statement → `BankAccount`. Senders and counterparties become lightweight
   `Organization` or party entities.
2. **Matching**, strongest first:
   - Scoped identifier (IBAN, policy number, VIN or plate, passport number,
     Steuer-ID, customer number with issuer): exact match links automatically.
   - Anchor persons: name, alias and birth date against setup anchors.
   - Name-only matches never merge automatically. Create a new entity and a
     "Same as X?" quick check. Merges remain reversible.
3. **Time.** Validity comes from the document (issue/expiry, pay period, policy
   term). Document date is the evidence time. A newer document closes the older
   interval; nothing is overwritten.
4. **Order independence.** Resolution is idempotent and re-runs for an entity
   when a new observation touches it, so a shuffled import order yields the same
   graph.
5. **Conflicts.** Overlapping incompatible single-value assertions mark both
   sides `check_suggested`.
6. **Household growth.** A non-anchor person who is the subject of 3 or more
   documents triggers a proposal: "Add Lena to your household?" Otherwise they
   remain a party.

### Confidence and review

Assertion columns: `confidence` (0–1), `confidence_source` (`checksum`,
`typesafe`, `openai_grounded`, `user`), `review_state` (`unreviewed`,
`check_suggested`, `reviewed`).

- Below the threshold → `check_suggested`. The initial threshold is a
  conservative engineering choice, not measured accuracy.
- The threshold is tuned from the user's own review outcomes: confirmations
  without edits allow it to drop; corrections raise it.
- The UI never shows a numeric score. It says "Worth a quick look".
- **Quick checks** groups `check_suggested` items per entity ("Your car: 2 things
  worth a look"), not one notification per fact.
- **First use** of an `unreviewed` fact (copy, form fill, agent read) shows an
  inline confirmation. Confirming sets `reviewed`.
- Search, profile and agent readers always expose review state.

## Initial setup and dump UX

### "Who are you?"

Conversational, one question per screen in the existing onboarding shell
(`ONBOARDING_*` geometry, fingerprint art that draws further with each answer).
Warm, not bureaucratic.

1. "Hi. Who are you?" — full name; optional former or maiden names as aliases.
2. Birth date.
3. "Who lives with you?" — names and role (partner, child, other). Skippable.
4. Anchors are stored as `Person` entities with `review_state = reviewed` and
   `confidence_source = user`. Editable later.
5. "Nice to meet you, Max. Now give ME everything." — choose or drop folders, or
   skip. Setup never waits for the import.

### Dump summary

No per-file confirmation for a setup dump. One summary instead: file count,
duplicates, unsupported files, the request estimate (TypeSafe decisions,
eagerly extracted documents, upper bound of OpenAI gap-fill calls), and what is
sent to TypeSafe and OpenAI. No currency amount is promised.

### Growing constellation

- Nodes are entities, not documents.
- At most about 30 nodes: the user, household, then the most-linked entities. The
  rest collapse into family clusters ("Invoices · 212").
- New nodes fade in; `check_suggested` nodes use the warning outline.
- Reuses Knowledge map geometry and motion tokens; respects reduced motion.
- An honest completed-work bar above the map (existing `progress_bar` rules).
- A one-line text ticker below the map with the latest discovery, so the progress
  is readable without the graphic and by assistive technology.
- Quota pauses appear on the map with a Resume action, never as a stuck bar.
- The dump continues in the background; the sidebar shows a compact progress
  item; the Imports page keeps per-file failures and retries.

All UI follows `docs/design-system.md`: shared tokens, `type_style`, 8 px radius,
ME Outline icons. Any new visual role updates the central tokens and docs.

## Cost control

- Batch allowance on top of the existing per-file allowance. Exhaustion pauses the
  queue visibly with an explicit Continue.
- Per-stage concurrency: Normalize 2 workers (CPU-bound local OCR); Classify and
  Extract 4 (TypeSafe HTTP); OpenAI gap fill 1. Throughput claims only from
  release-build measurements.
- The stage cache means registry or guidance updates never repeat OCR.
- These are request and reported-token controls, not a guaranteed currency cap,
  as documented in import-reliability.md.

## Errors

Reuse `ImportFailure` and its typed kinds. One failing file never blocks the
batch. Quota, rate-limit and authentication errors pause the queue. Stale runs
cannot write. Cancelling and vault lock stop active work; saved stage output is
kept for resume.

## Privacy

- Classify sends TypeSafe the capped document text and anchor names only, never
  the birth date.
- Credential sources and 1Password exports remain excluded.
- The dump summary states plainly what goes to TypeSafe and to OpenAI.
- No real personal data in logs, fixtures or tests.

## Migration (schema 15)

- New tables `intake_envelope` and `stage_run` (content hash, stage, version,
  encrypted output, usage).
- New column `source.document_type`; assertion columns `confidence`,
  `confidence_source`, `review_state`. Existing user-authored assertions become
  `reviewed` / `user`; existing extraction-derived confirmed assertions become
  `reviewed`.
- Existing documents are backfilled through Classify and Resolve; saved
  extraction steps are reused where their cache identity still matches.
- `ImportStage` maps onto the new stages so the Imports page keeps working.

## Future sources

- **Scanner:** a watched folder or ICA scanner adapter that produces envelopes.
- **Mail forwarding:** needs a receiving address and therefore the backend; it gets
  its own design. Its envelopes carry body, attachments and sender metadata.

Neither requires pipeline changes.

## Testing

All fixtures are synthetic.

- Candidate generators: MRZ check digits, IBAN, Steuer-ID, dates, amounts,
  label/value pairing.
- Registry validation and slot selection with fake `Decisions`.
- Classify thresholds and lazy fallback.
- Resolve: identifier matching, name-only non-merge, temporal supersession,
  conflicts, household proposal threshold, confidence → `check_suggested`.
- Order independence: shuffled import order produces an identical graph.
- Dedup, resume, stale runs and stage-version bumps.
- Schema 14 → 15 migration.
- One opt-in live TypeSafe test on a tiny synthetic passport and payslip.
- Gallery examples for the setup flow and the constellation, inspected at normal
  and minimum window sizes.

## Out of scope

ZIP and mailbox unpacking, photo-library filtering, mail forwarding backend,
visual-model fallback for unreadable scans, external web research, and
cross-device sync of pipeline state.

## Build order

1. Envelope, stage trait, queue and `stage_run` cache; move existing stages in.
   Add the core anchor API (user, aliases, birth date, household) here, because
   Classify and Resolve depend on it; its setup UI follows in step 6.
2. Folder walk, dedup and dump summary.
3. Classify with the type registry.
4. Local candidate generators and TypeSafe slot selection; OpenAI gap fill.
5. Resolve, confidence and review states, Quick checks, first-use confirmation.
6. "Who are you?" setup and the growing constellation.
