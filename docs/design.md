# ME. design

The required visual contract is [the design system](design-system.md), backed by
`apps/desktop/src/design_system.rs`. Read it before changing any UI.

**ME.** is a personal data filter. The interface is English; source data keeps its
original language.

- Left sidebar, 192 px: identity, Search, Logins, Review, Browser, Knowledge, Imports, Settings.
- Light cool surfaces, fine borders, graphite type, restrained indigo accents.
- Geist for navigation and labels; Geist Mono for stored values and small metadata.
- Start-page slogan: “It's about you. Your data. Your fingerprints.”
- A prominent search field accepts natural language, pasted files, and dropped forms.
- Recent details are visible immediately. Typing searches all collection kinds:
  notes, document filenames and enabled content, and imported credential titles
  and vault names. Items open their existing detail view; confirmed personal data
  retains its direct copy action and source. AI field matching adds personal-data
  results without removing local matches. Local search works without AI.
- Form results distinguish found, missing, and conflicting values. Export to Codex
  follows one concise review of the form and values being shared.
- Review and Browser use short lists and a clear expandable directory tree.
- Knowledge is a persistent honeycomb map of stored data and its source attribution.
  Related cells stay nearby; saved positions survive refresh. See [the map contract](knowledge-map.md).
- Keep secondary workflows out of the home screen until requested.

See [current behavior and validation](lean-ui.md).


Logins provides a dedicated, alphabetically sorted list and inline detail pane.
All includes active and archived Login entries; Favorites and Archive narrow it.
Other imported 1Password categories remain available through Search and Browser.
Selecting a login from global Search opens this same Logins page. See
[login import and editing](1password-import.md) for the data-preservation contract.
