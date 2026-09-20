# ME.

**It's about you. Your data. Your fingerprints.**

ME. is a native personal information app for macOS and Linux, built with Rust
and GPUI. Keep documents and details in an encrypted local vault, understand
incoming files, review the evidence, and find information when you need it.

**Development preview.** macOS builds and synthetic regression tests have been
verified. Linux is a target, but its desktop runtime has not yet been validated.
This is not a security-audited release, and document interpretation can be wrong
or incomplete.

## License and commercial use

Copyright © 2026 David Joos ([@DoktorDaveJoos](https://github.com/DoktorDaveJoos)).

**Source available for inspection; a separate paid license is required for use.**
ME.'s original code and assets are governed by the [ME. Source Inspection
License](LICENSE). It permits reading and downloading the source for inspection.
It does **not** grant permission to build, run, modify, redistribute, embed,
resell, or offer ME. as a service, including for personal, internal, educational,
or noncommercial use. Those activities require a separate written paid license
from the copyright holder.

This is **not an open-source license**. Public access to the repository does not
make the software free to use. Rights granted by applicable law, GitHub's terms,
and third-party licenses remain unaffected. See [third-party notices](THIRD_PARTY_NOTICES.md).
Contact the repository owner to discuss licensing; no purchase or license grant
occurs merely by viewing the repository. External code contributions require a
separate agreement before acceptance so commercial licensing rights remain clear.

## What works today

| Area | Current behavior |
| --- | --- |
| **Search** | Find saved details and documents, copy values, inspect sources, and match a form against confirmed information. |
| **Review** | Accept, correct, or reject extracted suggestions and answer questions about uncertain values. Conflicts remain visible. |
| **Browser** | Browse encrypted originals and organize documents in folders. Export an original when explicitly requested. |
| **Imports** | Drop one or more files anywhere in the window, review the file list, and confirm intake. Follow each file through processing, retry failures, or stop work. |
| **Vault** | Password-based unlock, encrypted storage, note history, locking, backup and restore, and local 1Password `.1pux` import. |

Imports report meaningful stages: **normalization → interpretation → local
context → extraction → verification**. Two background workers process files
concurrently. Saved steps support explicit resume after failures and restarts. Overall and
per-stage bars count completed work. Provider errors name the failed stage; quota
or authentication failures pause the queue. Per-file allowances persist across
retries: 12 OpenAI calls, 24 TypeSafe requests and 180,000 reported tokens by default. Model suggestions carry source evidence and require review before
becoming confirmed facts.

The context stage currently uses local document guidance. Live web research,
visual model analysis of page images, and universal document understanding are
not implemented. See [the import pipeline and research](docs/import-pipeline.md)
for the architecture, progress model, and remaining accuracy work.

### Document support

- PDF text and scans; JPEG, PNG, TIFF, and BMP images.
- Additional HEIC/HEIF, WebP, and GIF image handling on macOS.
- DOCX, ODT, RTF, and UTF-8 text such as TXT, Markdown, CSV, and TSV.
- EML email with MIME headers, decoded body text, and supported attachments.
- Other files can be preserved as originals; interpretation support varies.

OCR and format parsing run locally. macOS uses PDFKit, Vision, and ImageIO;
Linux uses external Poppler, Tesseract, and UnRTF tools. The Office parsers read
text and tables; embedded images and complex layout interpretation remain
limited. Supported attachments, file limits, and failure behavior are documented
in [document processing](docs/document-processing.md) and [email import](docs/import-pipeline.md).

## Privacy and AI processing

ME. stores vault data in SQLCipher and separately encrypts original files.
Background work keeps storage, cryptography, OCR, and model calls off the UI
thread. Backups require the original password; there is no password-recovery
service. Exporting a document writes an unencrypted copy at your chosen location.

The current app requires Codex installation and ChatGPT sign-in during setup.
**Automatic AI analysis is enabled by default:** supported imports are read
locally and their extracted content is sent to TypeSafe AI for typed decisions
and to OpenAI through the signed-in ChatGPT account for extraction. This consumes that account's available usage. Disable automatic analysis
in Settings to require manual release for analysis. Disabling it cannot retract
content already sent. ME. does not ship a shared API key or use a paid OpenAI API fallback. TypeSafe
requires your own API credential and can incur separate charges. See the
[TypeSafe research, recovery design and spending limits](docs/import-reliability.md).

Credentials imported from 1Password remain local and are excluded from AI
analysis and document content search. Optional access through the read-only MCP
bridge requires a separate vault sharing grant. Form handoff to Codex also has
its own review step.

Vault encryption does not guarantee that RAM, swap, clipboard history, operating
system caches, or external provider state contain no plaintext. Read the
[vault limitations](docs/desktop-vault.md), [AI integration](docs/codex-integration.md),
and [1Password import behavior](docs/1password-import.md) before using real data.
Never put personal documents, vault backups, credentials, or diagnostic excerpts
containing private information in public issues or pull requests.

## Development

These instructions are for the copyright holder and licensees whose separate
written agreement permits development. They do not extend the [license](LICENSE).

The workspace pins **Rust 1.98.1** in `rust-toolchain.toml` and **GPUI 0.2.2**.
Install Rust using [rustup](https://rustup.rs/), plus Python 3 for the design checks.

### macOS

Install full Xcode and select it with `xcode-select`. The build needs Apple's
Metal compiler as well as the Swift compiler used by the local document helper.
If Xcode reports a missing Metal toolchain, install its component:

```sh
xcodebuild -downloadComponent MetalToolchain
```

Configure TypeSafe locally before analyzing imports:

```sh
cp .env.example .env
chmod 600 .env
# Edit .env locally and set TYPESAFE_API_KEY; never commit the key.
```

The app reads this private file at runtime. `ME_TYPESAFE_ENV_FILE` can select a
different private file; `TYPESAFE_API_KEY` can also be set in the launch environment.
For distributed builds, configure a private path explicitly. The bundle never
contains `.env`.

Run the optimized app:

```sh
./scripts/cargo run --release --locked
```

Or build a development app bundle:

```sh
./scripts/bundle-macos
open target/release/ME.app
```

Development bundles include **Settings → Development → Wipe data**. Confirming
clears this vault's files, notes, credentials, extracted data, search index and
import history so the same files can be uploaded again. It keeps your password,
preferences and ChatGPT connection; original files and external backups are
untouched. Active work is canceled and disconnected before the reset. This is a
local reset, not secure erasure of disk sectors or provider history.

The control is included in debug builds and release builds explicitly compiled
with `--features me-app/development-tools` (enabled by the development bundle
helper). Ordinary `cargo build --release` builds omit both the UI and reset API.

Pass `debug` to the bundle script for `target/debug/ME.app`. Bundles are signed
locally for development; they are not Developer ID signed or notarized.

### Linux

GPUI's Wayland and X11 backends are enabled. A working graphics driver and
development libraries are required. The following Debian/Ubuntu packages are a
starting point; names vary by distribution and a complete Linux build/runtime
still needs validation:

```sh
sudo apt install build-essential pkg-config clang libclang-dev cmake \
  libfontconfig1-dev libfreetype6-dev libxkbcommon-dev libxkbcommon-x11-dev \
  libwayland-dev libvulkan-dev libxcb1-dev libxcb-render0-dev \
  libxcb-shape0-dev libxcb-xfixes0-dev libssl-dev \
  poppler-utils tesseract-ocr tesseract-ocr-deu tesseract-ocr-eng unrtf
./scripts/cargo run --release --locked
```

The document processor expects its Linux tools in `/usr/bin`. They are installed
separately, not downloaded automatically by ME.

### Checks

```sh
./scripts/check-design-system
python3 scripts/test-design-system.py
./scripts/cargo fmt --all -- --check
./scripts/cargo check --workspace --locked
./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings
./scripts/cargo test --workspace --locked
```

The regular suite uses synthetic data. Tests requiring a real Codex installation
or signed-in model access are ignored by default; live model tests require an
explicit `ME_CODEX_TEST_HOME`. Do not point test tools at a personal vault.
The current GitHub Actions workflow checks the design contract; it does not
replace the full local Rust suite or platform verification.

The Cargo helper uses a project-local toolchain in `.tools/` when present,
otherwise Cargo from `PATH`. Keep `Cargo.lock` committed. Build output, local
toolchains, vaults, backups, and credentials belong outside version control.

### Repository layout

```text
apps/desktop/          Native GPUI interface, assets, and platform integration
  packaging/           Desktop packaging; currently macOS bundle metadata
crates/me-core/        Encrypted vault, facts, sources, migrations, and import state
crates/me-documents/   Local document parsing, OCR helpers, and synthetic fixtures
crates/me-agent/       Codex integration, extraction pipeline, and read-only MCP bridge
crates/me-diagnostics/ Bounded local operational diagnostics
docs/                 Product notes, architecture, design, and behavior
scripts/              Repository-wide build helpers and design checks
.github/workflows/    GitHub Actions checks
Cargo.toml            Root Rust workspace; desktop is the default member
Cargo.lock            Shared locked Rust dependencies
```

Run the commands above from the repository root. The desktop Cargo package
remains `me-app`, and build output stays under `target/`. Future clients belong
in `apps/ios` and `apps/browser-extension`; the Axum backend belongs in
`services/api`. Create those directories when their implementations begin.

All UI changes follow the [shared design system](docs/design-system.md).
[AGENTS.md](AGENTS.md) describes development constraints.

## Product direction

The [account, sync, and login decisions](docs/architecture/2026-09-20/ACCOUNTS-SYNC-AND-LOGINS.md)
record the planned mandatory ME account, independent desktop/Chrome/iPhone clients,
end-to-end encrypted sync, and login matching. These are future capabilities;
the current application remains a local vault. The
[backend decision](docs/architecture/2026-09-20/BACKEND-OPTIONS.md) selects Rust,
Axum, and Tokio; the backend is not yet implemented. The
[accepted repository and release plan](docs/architecture/2026-09-20/REPOSITORY-AND-RELEASES.md)
keeps one repository with independent desktop, browser, iPhone, and API releases.
The desktop layout migration is complete; product release pipelines remain future work.

## Status and remaining work

The encrypted vault, import pipeline, evidence checks, review, and native UI are
implemented. Synthetic regression coverage is not a guarantee that every real
letter, insurance document, email, or payslip will be interpreted correctly.

Remaining work includes broader document evaluations, Linux runtime validation,
security review, production signing and packaging, and release performance
measurements. Device sync, embeddings, and tray integration are future work.
Earlier research in `docs/architecture/` provides design context; the dated
product decisions above supersede conflicting historical assumptions. The current
README and root license define the published project's status and licensing.
