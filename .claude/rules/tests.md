---
paths:
  - "tests/**/*.rs"
  - "tests/fixtures/**"
  - "examples/*.py"
---

# Test conventions

All tests are integration tests under `tests/`; there are no `#[cfg(test)]` modules in `src/`. `cargo test` requires `python3` on PATH because the fixtures are real child processes.

- **Name tests as full behavioral sentences** describing the invariant under test, e.g. `router_payload_excludes_task_text_and_filenames`, `evaluation_does_not_equate_end_turn_or_clean_diff_with_success`. Keep that style; a test named after the function it calls loses the point of the file.
- `tests/cli_e2e.rs` drives the **real binary** via `CARGO_BIN_EXE_orochi` against `tests/fixtures/mock_acp.py`, with a `tempfile::TempDir` workspace holding its own `config.toml`, `--data-dir` and `--cwd`. Fixture behavior is selected through `MOCK_BEHAVIOR` / `MOCK_MODELS` / `MOCK_LOG` env vars — add a behavior to the fixture rather than a new mock.
- `tests/router_acp.rs` drives routing advisers as `routing_only` fixture agents (`MOCK_BEHAVIOR=adviser`, `MOCK_VOTES`, `MOCK_COUNTER`); `tests/session_collaboration.rs` and `tests/discovery.rs` use generated shell executables. Never contact a real provider, require a login, or spend quota from a test.
- `tests/mcp_oauth.rs` drives the MCP sign-in against `tests/fixtures/mock_oauth.py` (an MCP server that is its own authorization server, on loopback); the test itself plays the browser by following the authorization URL. It checks PKCE and `state`, so never weaken those to make it pass. Each `MOCK_OAUTH_*` switch breaks one rule of the MCP authorization specification; add a switch rather than a second fixture. Credentials in tests always go through `oauth::Vault::file` — no test may touch the keychain of the machine it runs on.
- Never call `std::env::set_var` in a test: tests share one process and threads, and library code reads the environment (PATH lookups) concurrently. Behavior that depends on an environment variable is tested through the binary in `cli_e2e.rs`, with the variable set on the child `Command`.
- File split: `core.rs` = cross-cutting invariants, `adaptive.rs` = learning/quota/calibration, `discovery.rs` = CLI + adapter resolution, `cli_e2e.rs` = end-to-end binary behavior, `router_acp.rs` = routing advisers, `session_collaboration.rs` = `collaborate`, `mailbox.rs` = agent-to-agent messages (library, MCP server via the binary, concurrent runs with `MOCK_BEHAVIOR=mailbox_chat`).
- `tests/adaptive.rs` shells out to `python3 -m unittest discover -s tests -p test_quota_terminal.py`, so `src/quota_terminal.py` stays covered by `cargo test`.
- Scripts in `examples/` that touch a real agent (`live_e2e.py`, `gateway_live_e2e.py`, `collect_benchmark.py`, `compare_live.py`) must report a missing CLI or failed discovery as **blocked**, never as passed, and must only send a task under an explicit `--execute` flag. `compare_live.py --validate` and its suite's reference solutions are driven from `cli_e2e.rs`, which also runs the whole harness against the fixture.
