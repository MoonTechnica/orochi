---
paths:
  - "README.md"
  - "docs/**/*.md"
---

# User-facing documentation

`README.md` and everything under `docs/` are written in **English** and serve as the user-facing spec. Japanese appears only as a literal where the text is about Japanese-language handling (a prompt the profiler has to recognize, say), glossed in English.

- These documents deliberately separate **implemented / verified against the real CLI / unverified**. Existing text says things like "awaiting verification against the real CLI", "unverified", "automated tests only" and "out of scope". Do not upgrade such a claim without evidence from an actual run, and do not delete a limitation to make a feature sound finished.
- Claims about routing accuracy, quota sources and provider behavior must stay attributable: heuristic estimates are labeled as Orochi's own, and provider-sourced rules link the official page they came from.
- `docs/design-draft.md` is the original draft spec, not a record of what ships. `docs/adaptive-routing.md`, `docs/session-collaboration.md`, `docs/cli-discovery-e2e.md` and `docs/real-validation-*.md` record what was implemented and measured, and are dated — add a new dated document for new measurements instead of rewriting past results.
- Keep the module table in `README.md` ("Layout") in sync when adding or renaming a file in `src/`.
