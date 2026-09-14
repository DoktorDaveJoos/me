# ME

**You. Your Data.**

A fast personal data app for **macOS and Linux**, built with **Rust + GPUI**.
Commercial, proprietary software. Windows is outside the product scope.

## Current state

This is the initial project scaffold: a GPUI window, an application menu and quit
shortcut, plus a separate platform-independent core crate. It does **not** yet
store personal data or implement a vault, encryption, accounts, imports, tray
integration, or AI. Do not use this scaffold to store real personal documents.

## Run

```sh
./scripts/cargo run --release
```

The helper uses the project-local Rust toolchain in `.tools/` when available and
otherwise uses Cargo from your PATH. `.tools/` and `target/` are excluded from Git.
With a normal Rust installation, `cargo run --release` works as well.

Rust is pinned in `rust-toolchain.toml`. GPUI is pinned to published version 0.2.2;
use the matching crate sources/docs, because Zed's main branch has newer APIs.
Commit `Cargo.lock` so application builds resolve the same dependency versions.

## macOS development bundle

```sh
./scripts/bundle-macos debug
open target/debug/ME.app
```

Omit `debug` for an optimized release bundle. The helper signs the bundle locally
for development; it does not perform Developer ID signing or notarization.

## Prerequisites

### macOS

- Rust (see <https://rustup.rs>).
- Full Xcode with the macOS components and Metal compiler, selected with
  `xcode-select`. The command-line tools alone may not provide the Metal tools.
  If the build reports a missing Metal Toolchain, install the official component:

  ```sh
  xcodebuild -downloadComponent MetalToolchain
  ```

### Linux

Both Wayland and X11 GPUI backends are enabled. A working graphics driver and
the development libraries required by GPUI are necessary. On Debian/Ubuntu,
start with:

```sh
sudo apt install build-essential pkg-config clang libclang-dev cmake \
  libfontconfig1-dev libfreetype6-dev libxkbcommon-dev libxkbcommon-x11-dev \
  libwayland-dev libvulkan-dev libxcb1-dev libxcb-render0-dev \
  libxcb-shape0-dev libxcb-xfixes0-dev libssl-dev
```

Package names differ by distribution; this dependency list and Linux runtime
behavior still need validation on the target machines. See the
[GPUI source](https://docs.rs/crate/gpui/0.2.2/source/) and
[Zed Linux development guide](https://zed.dev/docs/development/linux).

## Development checks

```sh
./scripts/cargo fmt --all -- --check
./scripts/cargo check --workspace --locked
./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings
./scripts/cargo test --workspace --locked
```

Use release builds for responsiveness measurements. Measure cold start,
activation of an already-running app, and vault unlock separately.

## Layout

```text
crates/me-app/   GPUI views, window lifecycle, and platform integration
crates/me-core/  UI-independent data and vault foundation
docs/mvp.md     Product scope and implementation order
scripts/cargo   Cargo helper with optional project-local toolchain
```

## Next

Implement Basic's encrypted vault and manually entered facts, then the
menu-bar/tray quick-access flow. Pro adds document extraction through OpenAI,
followed later by encrypted device sync. See [the MVP plan](docs/mvp.md).


## Bootstrap verification

- macOS Apple Silicon: debug build and actual GPUI window launch verified.
- Formatting and workspace Clippy checks pass.
- Release performance and Linux builds/runtime have not been measured or verified.
- Cargo reports future-compatibility notices in third-party dependencies
  `block` and `proc-macro-error2`; these do not prevent this pinned build.
