# ME: agreed direction

> Current account, independent-client, sync, and login requirements are recorded
> in [the September 20 decision](architecture/2026-09-20/ACCOUNTS-SYNC-AND-LOGINS.md).
> Those decisions supersede conflicting sequencing and product assumptions below;
> they do not claim the planned capabilities are already implemented.

> Historical product outline. The September 14/15 architecture in
> `docs/architecture/` supersedes the OpenAI-exclusive Pro assumptions below.
> The first desktop vault implementation is tracked in
> [desktop-vault.md](desktop-vault.md). The current [README](../README.md) and
> [source inspection license](../LICENSE) supersede historical pricing and
> licensing assumptions below: source is public, but use requires a paid agreement.

## Product

ME is a personal memory for everything the user wants to retain about themselves.
It takes over the work of organizing personal files and retrieves the information
or original documents needed for a task. The AI is the eventual primary interface.

Receipts, CVs, career certificates, identity documents, payment details, facts
and personal notes all belong here. No fixed Identity/Finance/Contact taxonomy is
required of the user. Give ME something; ask for it when it matters.

- macOS and Linux; no Windows target.
- Rust + GPUI; responsiveness is priority one.
- Commercial, source-available product; use requires a separate paid license.
  Pricing is not decided.
- Basic foundation first: freely named facts/notes, local encrypted storage,
  search, copy and documents. The UI must stay open for the AI-driven product.
- Pro next: OpenAI-powered extraction, review of new/conflicting facts, chat
  and retrieval using document content and the user's context.
- Videos and contract evaluation are out of scope.
- Device sync comes later and stores only encrypted vault content server-side.

## Core experiences

1. **Give ME something.** Drop a file or add a fact/note, without selecting a
   destination folder or category. Preserve the original and acknowledge intake
   immediately; extraction and indexing happen in the background.
2. **Ask ME for something.** Retrieve a passport number and its expiry, a purchase
   receipt, or the right CV. Answers identify their sources. Missing or conflicting
   data remains visible instead of being guessed or silently overwritten.
3. **Find what fits a task.** Given a job description, select the user's relevant
   existing certificates and return the actual documents with a short explanation.
   This requires contextual retrieval, not just filename search.
4. **Use it elsewhere.** Later, a browser integration can fill a visa application
   or another form using selected facts. Surface the chosen values and target page
   before submission; fill and submit are separate actions. This is a later product
   capability, not something the current desktop preview can do.
5. **Extend the personal vault.** Logins, passwords and payment details are a
   longer-term extension. They do not define the navigation or the initial scope.
   Retrieval should supply only information relevant to the user's current task.

## Implementation order

1. **Bootstrap** — Cargo workspace, pinned GPUI, launchable app shell and a
   minimal, category-free interaction preview.
2. **Vault foundation** — master-password unlock, reviewed encryption libraries,
   encrypted metadata and files, locking, backup and tested restoration.
3. **Basic data flow** — local profile with stable ID; typed facts and custom
   fields and notes; multiple accounts/documents; search, pinning, copy and editing.
   Structured types are internal capabilities, not mandatory UI buckets.
4. **Document intake** — PDF/image drop, durable encrypted copy, file preview,
   association with facts, duplicate handling and recoverable import jobs.
5. **Desktop access** — menu bar/tray, keyboard shortcut and optional autostart.
   Prototype these early on macOS and Linux; tray behavior, focus and shortcuts
   need separate Wayland/X11 validation. Keep the main window fully usable.
6. **Pro** — OpenAI extraction with source references; compact confirmation for
   new data; targeted review for conflicts. Keep manual and AI-confirmed facts
   in the same model. Cloud calls and import work never block the UI thread.
7. **Contextual assistance** — select documents for applications and other tasks;
   then prototype browser filling and deeper credential-vault features separately.
8. **Sync** — authenticated device enrollment, encrypted replication, version
   conflicts, deletion propagation and recovery.

## Data architecture

Keep UI code out of `me-core`. Plan for entities (person/company/account/document),
typed facts, arbitrary user-defined properties, notes, original files and relations.
Each extracted fact needs provenance (document/page/region), revisions, validity
and confirmation state. Support concurrent versions, such as multiple CVs or old
and current passports. Dates such as issue and expiry belong to the fact/document,
not to a forced UI category. Originals and derived text/indexes remain distinct.
Search indexes, extracted text, file names and previews are sensitive vault data too.
Choose the actual schema and encryption design before implementing storage.

Local profile IDs are separate from later authenticated account IDs. An entered
email is unverified. Local plan flags are not authorization for paid AI usage.
Paid Pro needs backend account/entitlement checks and rate limits. A shared
OpenAI secret must not ship inside the app. Laravel is a candidate for this
backend; it is not part of the desktop runtime.

Encrypted sync and cloud analysis have different trust boundaries. An AI proxy
handling plaintext documents processes their content, even if it doesn't persist
it. Resolve provider terms for sensitive document processing before public Pro.

## Responsiveness

Initial targets, to be measured on named reference devices in release builds:

- Already-running, unlocked quick-access window ready for input: under 150 ms.
- Local search at normal personal-vault scale: under 50 ms.
- Visual drop acknowledgement: under 100 ms; durable storage is a separate state.
- No blocking file I/O, password derivation, encryption, or cloud work in render
  or event handlers. Measure tail latency and frame stalls, not only averages.
- Idle app should sleep rather than continuously repaint or poll.

## Release gates

Before handling real data: recovery and corruption tests for persistence, no
secrets in logs, safe temporary-file handling, locking and clipboard behavior.
Before distribution: macOS signing/notarization, Linux packaging and runtime
testing on GNOME/KDE, including Wayland. Never describe the bootstrap as a
finished secure vault.
