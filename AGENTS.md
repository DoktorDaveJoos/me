# ME development

- Target macOS and Linux. Rust + GPUI is the chosen stack; do not substitute a
  webview or add Windows support without a product decision.
- Responsiveness is priority one. Keep blocking I/O, crypto and network work off
  the UI thread. Use release builds for performance measurements.
- `me-app` owns UI/platform integration; `me-core` owns platform-independent data
  and vault behavior. Use the API of the pinned GPUI version, not Zed main.
- The project is proprietary. Preserve `publish = false` and the license.
- Basic precedes Pro. See `docs/mvp.md` for the agreed scope and exclusions.
- Never log real personal data or secrets. Never ship the shared OpenAI key.
- The current scaffold is not an encrypted vault; do not imply storage/security
  features exist until implemented and verified.
- Use `./scripts/cargo` if Cargo is unavailable in PATH. Commit Cargo.lock.
- Before finishing Rust changes, run formatting, relevant compilation checks and
  Clippy. Add behavioral tests when storage or other meaningful logic is added.
- Keep `.tools/`, `target/`, personal data, and local configuration out of Git.
