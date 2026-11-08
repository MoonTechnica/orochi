# Real validation: decision models for classifying a request (2026-11-08)

The question was which decision model, if any, should label each request before routing —
what kind of work it is and how hard — in place of asking a coding agent in an ACP session of
its own (`docs/real-validation-20261015.md`: 815 tokens and a 7 s turn on the leanest path,
after a 31 s session start). A decision model answers typed questions (`choice`, `score`,
`noul`) with probabilities and writes no text. Measured on this machine (Apple M5 Max, 64 GB)
with llama.cpp `b12202` (`llama-server`, which gained `POST /v1/systemone` in
ggml-org/llama.cpp#29818, merged 2026-10-02), with no fine-tuning of any model.

## The requests

30 requests, 20 in Japanese and 10 in English, each with the kind of work and difficulty Orochi's
author expected: small edits, bug fixes, refactors, research, reviews, discussion, migrations,
tests, architecture, planning, documentation, analysis, creative work and feature work. **The
expected labels are Orochi's own judgement, not an independent benchmark, and 30 requests is a
small sample.** The questions were the classifier's labels written as System One questions: one
15-way `choice` for the kind of work, a 4-level `score` for difficulty, `noul`s for the
capability needs.

## Results

| | kind of work | difficulty, exact | difficulty, ±1 level | web need | browser need | median latency | download |
|---|---|---|---|---|---|---|---|
| Orochi's keyword profile (`profiler`) | 11/30 | 21/30 | 29/30 | 28/30 | 30/30 | — | — |
| Always "normal" / always "no" | — | 18/30 | — | 28/30 | 28/30 | — | — |
| Laya multilingual, Q8 (`andreilungeanu/laya-multilingual-GGUF`) | 11/30 | 18/30 | — | 27/30 | 26/30 | 12 ms | 357 MB |
| **kev-4b, Q4_K_M (`ggml-org/Kev-4B-GGUF`)** | **25/30** | 19/30 | **30/30** | 30/30 | 28/30 | 224 ms | 3.0 GB |
| lev, Q4_K_M (`ggml-org/lev-GGUF`), measured 2026-11-09 | 26/30 | 24/30 | 30/30 | 27/30 | 27/30 | 306 ms | 3.0 GB |
| Laya (English), Q8 (`ggml-org/Laya-GGUF`), measured 2026-11-09 | 18/30 | 18/30 | 30/30 | 29/30 | 27/30 | 20 ms | 449 MB |
| Julia-1, Q8 (`ggml-org/Julia-1-GGUF`), measured 2026-11-09 | 12/30 | 7/30 | 24/30 | 7/30 | 6/30 | 10 ms | 168 MB |

- **Laya** loaded and answered in 12 ms, but labelled the kind of work no better than the
  keyword profile, and its difficulty scores barely moved (about 1 on a 0–3 scale for every
  request). Asked instead as yes/no questions ("is something broken?", "is research asked
  for?"), it did worse: a crash report read `broken = 0.20`, a request to search the web
  `research = 0.15`. Its author reports 0.362 accuracy for the base checkpoint and 0.766 after
  fine-tuning; fine-tuning was ruled out, since it would have to be repeated for every model
  update and every change to Orochi's own questions (the model list `suited` chooses from
  changes whenever an agent's models do), and there are too few recorded turns here to learn
  from.
- **kev-4b** labelled the kind of work right 25 times. Four of its five misses went to
  `general` (adding a `--json` flag, in both languages; a rewrite from Python to Rust; taking
  screenshots in a browser), one to `investigation` (finding and fixing a flaky CI test). Its
  difficulty is a continuous score whose order was right — simple 0.39–0.79, normal mostly
  0.7–1.2, complex 1.17–1.48, extreme 2.45–2.58 — so rounding it read only 19 exactly; the cuts
  Orochi uses for kev (`decision::MODELS`: 0.6, 1.4, 2.2) come from these numbers and are
  provisional until more requests are measured. Its confidences were low (0.17–0.85) whether
  right or wrong.
- **lev**, measured the next day, labelled the kind of work 26 times and the difficulty exactly
  24 times (with Orochi's plain cuts, 0.5 / 1.5 / 2.5), and 19 of its 20 answers at confidence
  0.8 or more were right — on this sample a little ahead of kev-4b, and it became the model recommended on this machine.
- **Laya (English)**, measured the next day, did better than its multilingual sibling on the
  same Japanese requests (18 against 11); **Julia-1** called nearly everything a web and
  browser need (7 and 6 of 30 right), which the 0.8 threshold on capability needs exists for.
- **OpenJev** (`ggml-org/OpenJev-GGUF`) was not run: 19 GB at Q4 and CC-BY-NC.
- Loading kev-4b into a fresh `llama-server` took 8.7 s, cold. That is why the local server
  outlives the process that starts it (`decision/local.rs`).

## What changed because of this

- `src/decision.rs`: the first conversation or task typed at a terminal asks where the decision
  model runs and which one (see "Choosing" below); a local choice downloads llama.cpp and the
  GGUF there and then, checked against pinned SHA-256s and resumable, a cloud one takes its
  provider's key. Choosing runs one question before the choice is saved.
- `classifier::refine` asks the chosen decision model, and where it cannot answer keeps the
  keyword profile; asking a coding agent is `[classifier] enabled = true`, now off by default.
- An unsure `general` never replaces a more specific keyword label, which on these requests
  turns kev's four `general` misses back into the keyword profile's answer. Capability needs and
  flags are raised only at a probability of 0.8 or more, and a named model or preference only
  at 0.6; both thresholds are Orochi's own, not measured.

## Run end to end

`orochi decision use local` against a fresh data directory, with the two downloads served from
this machine: llama.cpp unpacked, the GGUF's SHA-256 matched, the supervisor started
`llama-server`, and the check question was answered 11.2 s after the start (load included).
`--dry-run` then labelled "ログイン画面でパスワードを入力するとクラッシュする。直して" ("the login
screen crashes when a password is entered; fix it") as `bug_fix / normal` where the keyword
profile said `implementation`, and "アプリ全体をReactからSvelteに移行して" ("migrate the whole app
from React to Svelte") as `migration / extreme`.

## Choosing (2026-11-09)

The choice became two steps at the user's direction: where it runs (this machine, the cloud, or
none), then which model there, each shown with what was measured of it here. Only this machine
is ever recommended (lev); nothing is in the cloud, where which provider sees the requests is
the user's to choose. In the cloud each model is reached through its own maker's API, never a
reseller, with that maker's key: Jev (`jev-1.14.0`) on TypeSafe's System One API, gpt-6-luna on
OpenAI's Decisions API, whose request and answers Orochi translates from and to System One. kev,
whose only hosted offering is OpenRouter's, is local only.

Unverified: the cloud paths (TypeSafe's `jev-1.14.0` over `https://api.typesafe.ai/v1/systemone`,
OpenAI's `gpt-6-luna` over
`https://api.openai.com/v1/decisions`) have been run against a fixture only, with no key for
either provider on this machine; whether they answer as the local kev does is not known. The Windows and Linux builds of llama.cpp are
pinned but were not run here.
