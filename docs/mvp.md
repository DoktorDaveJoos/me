# ME: agreed direction

## Product

ME is a personal data wiki. The main jobs are adding data with as little effort
as possible and retrieving exact values or original files when needed.

- macOS and Linux; no Windows target.
- Rust + GPUI; responsiveness is priority one.
- Commercial, closed-source product. Basic pricing is not decided.
- Basic first: manual facts, local encrypted storage, search, copy, documents.
- Pro next: OpenAI-powered extraction, review of new/conflicting facts, chat.
- Videos and contract evaluation are out of scope.
- Device sync comes later and stores only encrypted vault content server-side.

## Implementation order

1. **Bootstrap** — Cargo workspace, pinned GPUI, launchable app shell.
2. **Vault foundation** — master-password unlock, reviewed encryption libraries,
   encrypted metadata and files, locking, backup and tested restoration.
3. **Basic data flow** — local profile with stable ID; typed facts and custom
   fields; multiple accounts/documents; search, favourites, copy and editing.
4. **Document intake** — PDF/image drop, durable encrypted copy, file preview,
   association with facts, duplicate handling and recoverable import jobs.
5. **Desktop access** — menu bar/tray, keyboard shortcut and optional autostart.
   Prototype these early on macOS and Linux; tray behavior, focus and shortcuts
   need separate Wayland/X11 validation. Keep the main window fully usable.
6. **Pro** — OpenAI extraction with source references; compact confirmation for
   new data; targeted review for conflicts. Keep manual and AI-confirmed facts
   in the same model. Cloud calls and import work never block the UI thread.
7. **Sync** — authenticated device enrollment, encrypted replication, version
   conflicts, deletion propagation and recovery.

## Data architecture

Keep UI code out of `me-core`. Plan for entities (person/company/account/document),
typed facts, provenance, revisions, validity and confirmation state. Search
indexes, extracted text, file names and previews are sensitive vault data too.
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
