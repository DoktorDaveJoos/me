# ME development

- Target macOS and Linux. Rust + GPUI is the chosen stack; do not substitute a
  webview or add Windows support without a product decision.
- Responsiveness is priority one. Keep blocking I/O, crypto and network work off
  the UI thread. Use release builds for performance measurements.
- Runnable clients live in `apps/`; shared Rust libraries live in `crates/`.
  The desktop package is `apps/desktop` and retains the Cargo name `me-app`.
  Run repository build and verification commands from the root.
- `me-app` owns UI/platform integration; `me-core` owns platform-independent data
  and vault behavior. Use the API of the pinned GPUI version, not Zed main.
- The project is source-available under the restrictive root `LICENSE`. Use
  requires a separate paid agreement. Preserve `publish = false`, the license,
  and third-party notices; public source is not permission for unrestricted use.
- Basic precedes Pro. See `docs/mvp.md` for the agreed scope and exclusions.
- Never log real personal data or secrets. Never ship the shared OpenAI key.
- An encrypted vault is implemented, but the app is not security-audited.
  Describe verified behavior and remaining security limits accurately.
- Use `./scripts/cargo` if Cargo is unavailable in PATH. Commit Cargo.lock.
- Before finishing Rust changes, run formatting, relevant compilation checks and
  Clippy. Add behavioral tests when storage or other meaningful logic is added.
- Keep `.tools/`, `target/`, personal data, and local configuration out of Git.

## UI design contract (required for every feature)

- Before adding or changing UI, read `docs/design-system.md` and
  `apps/desktop/src/design_system.rs`. Preserve the established ME. visual style.
- Use shared semantic colors, `space::*`, `radius::STANDARD`, and
  `type_style(Type::…)`; reuse the components in `theme.rs`. Do not add local
  hex colors, fonts, text sizes, spacing values or corner radii to feature views.
- Use only the bundled ME Outline family via typed `Icon` and `IconSize` values.
  New icons must match the 24×24, 1.5 px round-stroke asset contract.
- Rectangular controls, cards and dialogs all use the same 8 px radius. Only
  status dots and pill toggles are fully round. Use Geist for UI and Geist Mono
  for stored values/technical metadata; keep type size and leading together.
- New visual requirements must reuse a role or update the central token,
  documentation and all affected components together. Never fix a single screen
  with a nearby one-off value. Keep layout geometry distinct from spacing tokens.
- Run `./scripts/check-design-system`, `python3 scripts/test-design-system.py`,
  formatting, compilation and Clippy before finishing. Visually inspect changed
  screens at the normal and minimum window sizes; report any unverified states.
  Do not weaken the guard to make a new screen pass.
