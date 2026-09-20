# ME repository layout and independent releases

Date: 2026-09-20.
Status: accepted layout and release model. The desktop directory migration is
implemented. Future clients, the API, and product release workflows remain planned.
Rust + Axum + Tokio is the confirmed backend choice. One canonical checkout and
one GitHub repository hold the product family.

## Repository decision

Keep the canonical checkout at /Users/david/Workspace/me and retain one GitHub
repository, one main branch, and one history. Organize runnable products separately
from reusable libraries. Release each product independently from tagged commits.

The GPUI application lives in apps/desktop, with its own assets, examples, and
packaging. Core, agent, document-processing, and diagnostic libraries remain in
crates. Root tooling and documentation serve the entire repository.

## Target layout

    me/
      apps/
        desktop/             Existing GPUI app for macOS and Linux
          packaging/         Desktop bundle and installer definitions
        ios/                 iPhone app and AutoFill extension target
        browser-extension/   Standalone Chrome extension
      services/
        api/                 Rust + Axum + Tokio cloud backend
      crates/
        me-core/             Existing local vault and domain behavior
        me-agent/            Existing provider and agent integration
        me-documents/        Existing document processing
        me-diagnostics/      Existing diagnostic implementation
        me-protocol/         Future portable sync/API message definitions
      infra/                 Deployment definitions and local service setup
      docs/                  Product, architecture, design, and release notes
      scripts/               Repository-wide build and verification commands
      .github/workflows/     CI and product-specific release workflows
      Cargo.toml             One Rust workspace
      Cargo.lock             Locked Rust dependencies
      rust-toolchain.toml
      README.md
      AGENTS.md
      LICENSE

Future directories are created when they contain real implementations. The tree
does not imply placeholder apps, a functioning API, or new packages already exist.
Extract additional shared crates only when concrete consumers justify them.

## Boundaries and languages

- Desktop: Rust + GPUI, with necessary platform integration.
- Cloud: Rust + Axum + Tokio, with internal modules for accounts, devices, sync,
  objects, and entitlements. It may run API and worker processes from one codebase.
- iPhone: native Apple interface and AutoFill integration, with a portable Rust
  library where sharing is useful. Swift bindings can be generated using UniFFI;
  binding technology and exact local-vault integration still require validation.
- Browser: TypeScript/JavaScript, HTML/CSS, and browser APIs. Rust can supply
  portable components through WebAssembly if that proves useful; existing
  SQLCipher and desktop-specific crates cannot simply be compiled unchanged.
- Shared protocol: transport definitions, validation rules, and compatibility
  fixtures, independent of GPUI, SQLCipher, or Apple/browser frameworks.
- Client vault logic can hold decryption capabilities; the API must not gain those
  capabilities through a broad dependency on the client vault crate.

Accurate product wording is “Rust core and cloud backend, with native clients.”
The whole product family should not be called entirely Rust if it contains Swift
and TypeScript. Existing macOS document processing already includes a Swift helper.
Language choice is not a substitute for tested behavior or a security review.

## Workspace and local development

Keep one root Cargo workspace for the Rust desktop, API, and shared packages.
Cargo supports members at these different paths with one lockfile. Build specific
packages when working on a product; test the appropriate dependency set in CI.

The browser extension keeps its JavaScript package manifest and lockfile in its
own application directory until multiple JavaScript packages justify a workspace.
The iPhone project and its extension targets live together under apps/ios. Xcode
and JavaScript tooling coexist with Cargo in the repository; Cargo does not replace
their packaging systems.

Use one local clone. Separate Git worktrees are optional for simultaneous work;
they are not independent repositories or alternative canonical staging copies.
Existing work/ directories in the older ChatGPT workspace remain historical.

Do not create permanent desktop, ios, browser, or backend branches. Use short-lived
feature branches and pull requests into main. A protocol change can update server,
clients, and compatibility fixtures in one reviewable change.

## Independent releases

Illustrative tag conventions and distribution targets:

| Product | Example tag | Distribution |
| --- | --- | --- |
| Desktop macOS/Linux | desktop/v0.2.0 | Product-specific installers, GitHub release assets, future updater feed |
| Chrome extension | extension/v0.1.0 | Chrome Web Store |
| iPhone | ios/v0.1.0 | TestFlight, then App Store |
| Cloud API | api/v0.1.0 | Immutable container image, staging then production |

Each tag points at a complete repository commit, while its workflow builds and
publishes only the named product and that product's required dependencies. All
four products may have different versions and release dates. Shared libraries are
compiled into the selected product from the tagged commit; an internal library
change does not publish every client automatically.

Internal crates retain the shared workspace package version. The desktop package
owns its explicit version in apps/desktop/Cargo.toml, currently 0.1.0. Its macOS
bundle metadata remains in apps/desktop/packaging/macos/Info.plist; release tooling
must keep these versions consistent. Future products own their API package version,
extension manifest version, or iOS version/build numbers. Protocol versions and
database schema versions are separate concepts.

GitHub Releases can hold separate entries and assets for prefixed tags in the same
repository. Their automatically generated source archives contain the repository
snapshot, not just the product subdirectory. End users receive the installer or
store package for their product. The source-available root license remains in force.

## CI and release process

1. Pull requests run repository checks and tests for affected products. Shared
   protocol changes exercise all consumers and cross-client compatibility tests;
   use dependency awareness rather than only looking at the changed app folder.
2. Keep a reliable required aggregate check so conditionally skipped jobs do not
   leave branch protection waiting. Toolchain/build/workspace changes trigger
   broader checks. Platform-specific jobs use supported macOS/Linux runners.
3. Merge to main after checks. Merge itself does not publish all products.
4. A product-specific tag or selected release workflow builds from an immutable
   commit, validates product versions, tests, signs, and packages that product.
5. Keep separate release permissions and production environments for Apple,
   Chrome, desktop signing, and cloud deployment. Credentials remain outside Git.
   Production deployment and store publication are distinct from compiling a tag.
6. Record source commit, artifact checksums/image digest, and product/protocol
   versions. Review release notes for the actual product scope.

The present repository only has a design-system GitHub workflow and a local macOS
development bundler. Full CI, notarized distribution, store publication, and cloud
deployment pipelines are future work, not capabilities established by this note.

## Compatibility is what permits independent releases

The API and encrypted record formats must remain compatible with supported older
clients. Mobile/store reviews and users who update late mean releases cannot
assume every client upgrades together.

Prefer additive protocol changes and explicit capability negotiation. Deploy a
compatible backend before clients start using a new capability. Preserve unknown
fields and distinguish unsupported required capabilities from empty data. Test
older supported clients against the new server and current clients against
supported server behavior.

Database changes use a staged compatibility strategy so an old server version is
not restored against a schema it cannot understand. Define supported client
versions, deprecation windows, and read-only/update-required behavior for genuinely
incompatible clients. Product version numbers are not a substitute for this policy.

## Desktop migration — 2026-09-20

Moved the desktop package from crates/me-app to apps/desktop, preserving its Cargo
name me-app and binary name me. Moved desktop packaging to apps/desktop/packaging.
Shared crates, root manifests/toolchain, documentation, and cross-project scripts
remain in place. The root Cargo workspace still selects the desktop by default.

Updated Cargo membership/path dependencies, bundle asset/license paths, the
design-system guard, development instructions, and documentation links. The
existing GitHub design-system workflow continues to invoke the root scripts.
The desktop version is now explicit rather than inherited from internal crates.

Preserved local uncommitted work, the root lockfile and license, application
identity, bundle output paths, and vault locations. Future client/API directories
will be introduced with their implementations.

Validation completed on macOS:

- Formatting, all-target/all-feature compilation, and Clippy with warnings denied.
- The full default workspace test suite passed before and after the move; live
  provider tests remain opt-in and were not run.
- Design guard and its five tests passed; all 32 local links in the updated
  documentation resolve.
- Cargo metadata confirms five workspace members, the desktop default, shared
  dependency paths, the root license, and disabled package publishing.
- The release macOS bundle builds, passes ad-hoc signature verification, and
  contains the expected identity, version, and license assets. A temporary copy
  launched to the vault screen using a synthetic vault.
- The existing native startup harness passed its wrong-password, unlock, local
  write, and provider-check assertions. Its workspace was visually inspected at
  1120 × 820 and 800 × 600 using the relocated production UI sources.
- A file-hash and permission comparison accounts for all 194 pre-migration source
  files, including 66 relocated files. Only the intended manifests, scripts, and
  documentation changed during this migration; existing local work is preserved.

Linux runtime validation, live provider tests, and production signing/notarization
were not part of this source-layout migration.

## References

- [Account and sync requirements](ACCOUNTS-SYNC-AND-LOGINS.md).
- [Backend selection and comparison](BACKEND-OPTIONS.md).
- [Cargo workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html).
- [GitHub Releases and tags](https://docs.github.com/en/repositories/releasing-projects-on-github/about-releases).
- [GitHub Actions workflow syntax](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax).
- [Chrome extension structure](https://developer.chrome.com/docs/extensions/get-started/tutorial/hello-world).
- [Chrome Web Store publication](https://developer.chrome.com/docs/webstore/publish).
- [App Store Connect build upload](https://developer.apple.com/help/app-store-connect/manage-builds/upload-builds/).
- [UniFFI language bindings](https://mozilla.github.io/uniffi-rs/latest/).
