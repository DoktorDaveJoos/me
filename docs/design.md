# ME. design

The required visual contract is [the design system](design-system.md), backed by
`crates/me-app/src/design_system.rs`. Read it before changing any UI.

**ME.** is a personal data filter. The interface is English; source data keeps its
original language.

- Left sidebar, 192 px: identity, Search, Review, Browser, Settings.
- Light cool surfaces, fine borders, graphite type, restrained indigo accents.
- Geist for navigation and labels; Geist Mono for stored values and small metadata.
- Start-page slogan: “It's about you. Your data. Your fingerprints.”
- A prominent search field accepts natural language, pasted files, and dropped forms.
- Recent details are visible immediately. Typing replaces them with matching data,
  never a conversational answer. Each row has a direct copy action and a source.
- Form results distinguish found, missing, and conflicting values. Export to Codex
  follows one concise review of the form and values being shared.
- Review and Browser use short lists and a clear expandable directory tree.
- Keep secondary workflows out of the home screen until requested.

See [current behavior and validation](lean-ui.md).
