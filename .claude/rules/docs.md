---
paths:
  - "README.md"
  - "docs/**/*.md"
---

# User-facing documentation

`README.md` and everything under `docs/` are written in **Japanese** and serve as the user-facing spec. Write in Japanese when editing them; code, identifiers and in-source comments stay English.

- These documents deliberately separate **implemented / verified on real hardware / unverified**. Existing text says things like「実機確認待ち」「未検証」「対象外」. Do not upgrade such a claim without evidence from an actual run, and do not delete a limitation to make a feature sound finished.
- Claims about routing accuracy, quota sources and provider behavior must stay attributable: heuristic estimates are labeled as Orochi's own, and provider-sourced rules link the official page they came from.
- `docs/design-draft.md` is the original draft spec, not a record of what ships. `docs/adaptive-routing.md`, `docs/session-collaboration.md`, `docs/cli-discovery-e2e.md` and `docs/real-validation-*.md` record what was implemented and measured, and are dated — add a new dated document for new measurements instead of rewriting past results.
- Keep the module table in `README.md` (「構成」) in sync when adding or renaming a file in `src/`.
