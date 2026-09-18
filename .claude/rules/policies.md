---
paths:
  - "policies/*.json"
  - "src/policy.rs"
  - "src/router/scorer.rs"
  - "src/learning.rs"
---

# Provider policy and scoring

`policies/{openai,anthropic,google}.json` are compiled in via `include_str!` and validated by `policy.rs` before use. Every struct is `#[serde(deny_unknown_fields)]`, so a new JSON key needs a matching field plus a validation rule, and `Registry::bundled()` must still pass `validate()`.

- **`ModelRule::pattern` is a case-insensitive substring matched against model IDs discovered over ACP.** It selects priors for an ID the agent reported; it must never be used to construct or suggest a model ID that was not discovered.
- **`success_prior`, `relative_tokens` and cache discounts are uncalibrated Orochi heuristics, not provider-published numbers.** Keep that distinction explicit in code comments and docs when you touch them.
- `success_prior` is a 4-element array indexed by `Complexity` (simple, normal, complex, extreme) — keep the order aligned with `Complexity::index()`.
- Policy updates replace the installed registry atomically only after the whole registry validates; a failed update must leave the previous version intact. Remote updates require HTTPS plus an explicit sha256 digest.
- Scoring order in `scorer::candidates` matters: capability flags → quota availability → reasoning/mode resolution → `policy.permits` hard constraints → `required_success` floor → cost. A candidate rejected by a hard constraint must never reappear later via the router, council or bandit.
- Candidate IDs are a truncated hash of (agent, model, reasoning, mode). They are the only candidate identity sent to an external router, and `learning`/`benchmark` join on them — don't change the derivation without migrating stored records.
- `learning.rs` and `benchmark.rs` share the EWMA, exploration and `resource_cost` functions so replay matches live routing. Change them together.
