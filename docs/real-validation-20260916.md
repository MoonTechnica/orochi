# Measurement and Session Collaboration Validation (2026-09-16)

## 1. Measuring selection accuracy

Added `examples/collect_benchmark.py`. It runs each candidate from the same starting files, preparing an independent ACP session, working directory and telemetry DB each time. Only complete pairs for which matching model, reasoning and mode, a real evaluation, and real usage were confirmed are written to the dataset. It alternates the execution order of the candidates and records failures, missing data and authentication errors.

```sh
python3 examples/collect_benchmark.py \
  --suite examples/paired-suite.json \
  --adapter-cache /path/to/existing/orochi/adapters \
  --output /path/to/new/measurement --execute
orochi benchmark --input /path/to/new/measurement/dataset.json
orochi benchmark-tune --input /path/to/new/measurement/dataset.json --training-cases 4
```

The coefficient search selects alpha, prior_weight and exploration using only the first half of the time series, and validates on the second half, carrying over the selection history. It prioritizes the number of successes; at an equal number of successes, it compares cost including the failure penalty. It does not change the configuration automatically. A test asserts that changing the validation labels does not change the coefficient selection.

### Real data

A Python interval-merge task was run 8 times each with Claude Haiku and with Codex gpt-5.6-luna / medium. The first round's task text was ambiguous about "touching intervals", so it was not used; the re-measurement, whose task text stated explicitly that "[1,4] and [5,9] are separate intervals", was used instead.

| Candidate | Success | Total tokens | Average tokens per run |
|---|---:|---:|---:|
| Claude Haiku | 8/8 | 869,494 | 108,686.75 |
| Codex gpt-5.6-luna / medium | 8/8 | 207,058 | 25,882.25 |

Below is a time-series replay on the same measured data, in which each strategy learned only from the results of the candidates it chose.

| Replay strategy | Success | Total tokens | Tokens per success |
|---|---:|---:|---:|
| Static | 8/8 | 207,058 | 25,882.25 |
| EWMA | 8/8 | 264,746 | 33,093.25 |
| Bandit (seed=42) | 8/8 | 264,746 | 33,093.25 |

The coefficients searched over the first 4 runs did not change from the defaults, and over the last 4 runs all three strategies had 4/4 successes and 105,234 tokens. **With this data, no advantage for EWMA/Bandit can be confirmed. The default coefficients have not been changed.**

These are repetitions of the same task family, not 8 independent tasks. This is a small-scale functional check; it does not mean a general performance comparison between Providers, nor that calibration is complete for all tasks. The tokens are the values each ACP adapter reported, not charges. They include differences in how different Providers report cache/usage.

Evidence: `.orochi/live-e2e/session-work/paired-unambiguous/{dataset,evidence}.json`, `analysis/{replay,tuning}.json`. The first round's diagnostic data is saved in `paired-live/`.

## 2. Selection bugs found by measurement

- A 429 embedded in ACP's `data.details` had become a plain `Other: Internal error`. It is now classified as RateLimit, without displaying the internal details.
- Even when a model was specified explicitly, the configuration of every model was tried, so a usage limit on an unrelated model made the whole connection fail. Now only the explicitly specified model is explored.
- `entirely` was picked up as `entire`, so a small function was classified as Complex. English word boundaries are now distinguished.

## 3. Quota

Verified the official `/usage` and the statusline of Claude Code 2.1.272 against the real CLI. The statusline's 5-hour and weekly percentages and reset times are now ingested.

Added the `claude_usage` and `antigravity_usage` native CLI probes. Using Python 3 / a POSIX PTY, they read only the official CLI's `/usage` display. They do not extract auth tokens or call a Provider's non-public APIs. They do not store the screen text or account names.

For Claude, the real quota was retrieved and saved both with the probe alone and with `orochi quota --refresh`. Ingesting a captured real statusline JSON with `quota-ingest` was also confirmed, in a separate verification DB. Antigravity is not signed in, so parsing its quota display against the real CLI is unverified. An unknown UI or a failure to retrieve is treated as unknown/unavailable, and an empty retrieval does not overwrite an existing valid observation.

```toml
[quota]
timeout_secs = 30
[[quota.probes]]
agent = "antigravity"
kind = "antigravity_usage"
command = "/absolute/path/to/agy"
```

Codex and Claude are auto-detected from the standard presets. Antigravity is auto-detected if `agy` is on PATH. So that `quota --refresh` does not start OAuth for an Antigravity that is not logged in, authentication is checked first with `agy models`. Model names are not guessed; a reading is used only when an explicit model ID and the remaining/used direction can be read.

## 4. Session collaboration

See [`session-collaboration.md`](session-collaboration.md). Three independent sessions of the same Agent and the same model, the review handoff, isolation of edits made during review, and evaluation after integration were confirmed with a fixture.

**Implement → review → integrate also succeeded with the real Claude Haiku.** It created an ASCII slug function and passed the final local evaluation. The implementer's, reviewer's and integrator's ACP session IDs are all different. The reviewer role is not subject to file evaluation, so its own outcome is `partial_success`, and the final deliverable is `success`.

Evidence is in `.orochi/live-e2e/session-work/council-live/report-final.json`; the deliverable is `council-live/result-final/integrator/slug.py`.

## 5. Antigravity / Gemini

- Installed Antigravity CLI 1.2.3 and the official ACP Server 1.1.1 into `.orochi/tools/antigravity/`.
- The ACP connection got as far as `Authentication required`. Task execution, model selection and evaluation need to be verified after a Google login.
- Gemini CLI was excluded from further support at the user's instruction. At the time the installation was checked, 0.59.0 had stopped because no API key was set; it was neither logged in nor run.
- Antigravity's official installer added the PATH of its dedicated install location to `.zshrc`, `.zprofile` and `.bash_profile`.

## 6. ACP gateway

`examples/gateway_live_e2e.py` connected the public `orochi serve` to the real Codex. Confirmed ACP initialize/new/prompt, 102 stream updates, the generated file, and a successful local evaluation. Evidence is in `.orochi/live-e2e/session-work/gateway-live.json`.

E2E in the Zed UI is awaiting explicit approval, because temporarily adding an external agent to the global settings was rejected by the auto-approval review. An example configuration has been created at `zed-gateway/zed-agent-entry.json`. A live-service test through this ACP client is not treated as equivalent to an IDE UI test.

## 7. Checks before adding role failover

- `cargo test --locked`: 66 Rust integration tests and the 4 Python quota-parser tests run from within them passed.
- `cargo clippy --locked --all-targets -- -D warnings` and `cargo fmt --check` passed.
- The cancellation test failed by exceeding the 2-second limit on the startup check, so it was changed to wait up to 10 seconds while checking the state. On a startup failure, it reaps the child process and prints the diagnostic log.
- `.env*` is excluded from a session's working copy, and participant IDs that collide when differing only in case, as well as the use of reserved directory names, are rejected. Added regression tests for isolation and exclusion.

The remaining external conditions are a Google login for Antigravity, approval to temporarily add the Zed setting, and configuring the HTTP Judge/Council connections to a real service. Further calibration on diverse tasks and multi-round collaboration with arbitrary recipients are remaining tasks beyond the scope of this small-scale measurement and fixed flow.

## 8. Additional implementation of role failover, including the coordinator

Added an optional `coordinator` role to `collaborate`. It updates the plan, decisions and open issues at the start and after each step finishes. In every role (coordinator, implementation, review and integration), when the specified candidate becomes unavailable, the role is handed over to an available Agent/model. The handover respects the candidate range, pinned selections, Policy, estimated success rate, quota and attempt limit.

`report.json` is saved atomically as resumable state with schema version 2. Partial replies are saved as each one is received, and the failed assignee's edited files, reply and verification results are passed to the next independent session. After every candidate has failed, or after a cancellation, `collaborate-resume` resumes from the incomplete steps. Completed steps are not re-run, and an explicit cancellation does not automatically launch a replacement AI.

HTTP Router/Frontier Judge/Council can now also be configured with up to 3 fallback endpoints. On an error such as 402/429 or an invalid reply, the same decision inputs are passed on, and the Council keeps one vote per member and a majority. The total budget, including the fallback candidates, is capped in advance. These HTTP failovers are fixture verification, and are kept distinct from verifying authentication and connectivity against a real HTTP service.

### Continuation check with the real Codex

A fixture caused insufficient credits for the coordinator, and the role was automatically handed over to the real, logged-in Codex. On the first try, a Codex CLI warning was mixed into the start of the reply, and strict JSON parsing failed. After fixing the parsing to validate and read the complete management-state JSON at the end, **it resumed from the saved report and completed all 7 steps**.

The real Codex served as coordinator 4 times, implementer once, reviewer once and integrator once, and both the local evaluation and an independent check verified that the integrated `result.txt` exactly matches `OROCHI_FAILOVER_OK` plus a single newline character. It was also confirmed that the original test working tree was unchanged.

This is a validation of **a simulated limit error → continued execution on the real Codex**, not a test that exhausted the credits of a real Claude account.

Evidence: `.orochi/live-e2e/session-failover/verification.json`, `result/report.json`, `before-resume.json`. For usage and what is stored, see [Session Collaboration](session-collaboration.md).

After the additional implementation, all 81 Rust tests and the 4 Python quota-parser tests called from within them passed. These include the cases of a Ctrl-C interruption of the coordinator, saving partial replies, resuming from the same step, and detecting a model-scoped limit while keeping the other models. Clippy (`--all-targets -- -D warnings`) and `cargo fmt --check` also passed.

## Official references

- [Claude statusline](https://code.claude.com/docs/en/statusline)
- [Antigravity /usage](https://www.antigravity.google/docs/cli/commands/usage)
- [Antigravity CLI installation](https://www.antigravity.google/docs/cli/install)
- [ACP Registry](https://github.com/agentclientprotocol/registry)
- [Zed external Agent configuration](https://zed.dev/docs/ai/external-agents)
