// Answers for the assistant and the routine settings, so `preview.html` shows them.
(() => {
  const now = Math.floor(Date.now() / 1000);
  const answers = {
    keeper: { running: true, status: { pid: 4242, started_at: now - 7200, version: "0.1.0" }, login: false, login_supported: true, tokens_today: 214000, daily_tokens: 300000, enabled: true },
    residents: [
      { thread_id: "r1", name: "assistant", home: "/data/residents/assistant", instruction: "You are my morning assistant: calendar, mail and CI.", access: "read", paused: false, created_at: now - 86400, next_at: now + 5400, routines: 2, asking: 0, working: false },
      { thread_id: "r2", name: "research", home: "/data/residents/research", instruction: "Keep an eye on new papers about agent scheduling.", access: "read", paused: true, created_at: now - 3 * 86400, next_at: null, routines: 1, asking: 0, working: false },
    ],
    routines: [
      { id: "f1e2d3c4-0000", thread_id: "r1", thread_title: "assistant", name: "Release watch", instruction: "Say what landed in the new release.", schedule: null, watch: "gh release list --limit 1", every_secs: 900, access: "read", notify: "always", catch_up: "skip", made_by: "agent", expires_at: now + 7 * 86400, paused: true, proposed: true, next_at: now + 900, last_at: null, last_skipped: null, last_state: null, tokens_today: 0 },
      { id: "a1b2c3d4-0000", thread_id: "r1", thread_title: "assistant", name: "Morning brief", instruction: "Look at my calendar, mail and last night's CI, and tell me what matters today.", schedule: "weekdays at 08:00", watch: null, access: "read", notify: "always", catch_up: "once", made_by: "user", expires_at: null, paused: false, next_at: now + 5400, last_at: now - 80000, last_skipped: null, last_state: "completed", tokens_today: 0 },
      { id: "e5f6a7b8-0000", thread_id: "r1", thread_title: "assistant", name: "CI check", instruction: "Check the CI runs and say if one failed.", schedule: "every 2h from 09:00 to 19:00", watch: null, access: "read", notify: "mention", catch_up: "skip", made_by: "agent", expires_at: null, paused: false, next_at: now + 3000, last_at: now - 4000, last_skipped: "budget", last_state: null, tokens_today: 41000 },
      { id: "c9d0e1f2-0000", thread_id: "r2", thread_title: "research", name: "Paper digest", instruction: "Find new papers and summarise the three most relevant.", schedule: "every mon,thu at 10:00", watch: null, access: "read", notify: "always", catch_up: "once", made_by: "user", expires_at: null, paused: true, next_at: null, last_at: now - 5 * 86400, last_skipped: null, last_state: "failed", tokens_today: 0 },
    ],
    routine_firings: [
      { id: 3, routine_id: "a1b2c3d4-0000", due_at: now - 80000, started_at: now - 79990, turn_id: "t1", skipped: null, turn_state: "completed" },
      { id: 2, routine_id: "a1b2c3d4-0000", due_at: now - 166400, started_at: null, turn_id: null, skipped: "late", turn_state: null },
    ],
    routine_preview: { words: "weekdays at 08:00", next: [now + 5400, now + 91800, now + 178200] },
    notices: [
      { id: 3, kind: "ask", thread_id: "r1", routine_id: "e5f6a7b8-0000", prompt_id: "p1", title: "CI check: wants to run gh run list", body: "It is declined by itself if nobody answers within 30 minutes.", mac: "skipped_window", phone: "sent", created_at: (now - 600) * 1000, read_at: null },
      { id: 2, kind: "report", thread_id: "r1", routine_id: "a1b2c3d4-0000", prompt_id: null, title: "Morning brief", body: "Two meetings today. CI is green. @you the invoice from Friday is still unpaid.\n\n(Notes updated: +2 −1 lines.)", mac: "off", phone: "failed", created_at: (now - 80000) * 1000, read_at: null },
      { id: 1, kind: "budget", thread_id: null, routine_id: null, prompt_id: null, title: "Routines have used 80% of today's token budget", body: "240000 of 300000 weighted tokens (routines.daily_tokens)", mac: "off", phone: "held", created_at: (now - 90000) * 1000, read_at: now * 1000 },
    ],
    notify_settings: {
      notify: { mac: true, level: "title", away_secs: 300, coalesce_secs: 60, quiet: "23:00-07:00", wake_for_approvals: false, kinds: ["ask", "report", "mention", "trouble", "scheduled", "budget", "keeper"], keychain: true, onesignal: { enabled: true, app_id: "8f3c-demo-app-id", url: "https://api.onesignal.com/notifications", api_key_env: "" } },
      routines: { enabled: true, min_interval_secs: 900, max_per_thread: 10, max_proposals: 5, proposal_days: 7, max_late_secs: 21600, daily_tokens: 300000, ask_secs: 1800 },
      residents: { root: "{data}/residents", notes_chars: 6000, brief_reports: 3 },
      key: "keychain",
      phones: [{ id: "3f9a1c0de2b74a65b3c1d0e9f8a7b6c5", name: "My iPhone" }],
    },
    resident_notes: { notes: "# Notes of assistant\n- prefers tea\n- the invoice is due Friday\n", previous: "# Notes of assistant\n- prefers tea\n", home: "/data/residents/assistant", instruction: "You are my morning assistant: calendar, mail and CI." },
    notify_test: { notice: 9, mac: "sent", phone: "sent" },
  };
  // The assistant's own conversation: a question answered from what it read, a routine's wake
  // and its brief, and a request that left the proposal above waiting.
  const ms = Date.now();
  const home = "/data/residents/assistant";
  let seq = 0;
  const item = (at, over) => ({ seq: ++seq, turn: "a1", turn_ordinal: 1, lane: 0, role: "lead", agent: "claude", model: "opus", kind: "agent_message", status: "completed", text: "", data: null, truncated: false, patches: 0, at, failed: false, turn_origin: null, provider: "anthropic", ...over });
  const residentThread = {
    thread: { id: "r1", title: "assistant", project: "assistant", branch: null, cwd: home, status: "idle", agents: [] },
    seats: [],
    files: [],
    items: [
      item(ms - 90000000, { kind: "user_message", lane: null, role: null, text: "When is the invoice from SOLID HOUSE due?" }),
      item(ms - 89990000, { kind: "tool_call", data: { tool_kind: "read", title: "Read", locations: [{ path: `${home}/mail/SOLID HOUSE invoice.pdf` }] } }),
      item(ms - 89980000, { kind: "tool_call", data: { tool_kind: "fetch", raw_input: { url: "https://www.example.com/billing/terms" } } }),
      item(ms - 89970000, { text: "**Friday, November 28.** The invoice's own terms say payment is due 14 days after issue, and it was issued on the 14th." }),
      item(ms - 10800000, { kind: "user_message", lane: null, role: null, turn: "a2", turn_origin: "routine", text: 'You are assistant.\n\nThis turn was started by a routine, "Morning brief" (due 08:00). Nobody is at the keyboard.' }),
      item(ms - 10790000, { turn: "a2", kind: "tool_call", data: { tool_kind: "read", locations: [{ path: `${home}/calendar/today.ics` }] } }),
      item(ms - 10780000, { turn: "a2", text: "Good morning. Two things today:\n\n- 10:00 viewing at the new flat\n- 15:00 call with the insurer\n\nLast night's CI passed on main. The SOLID HOUSE invoice is still unpaid and due Friday." }),
      item(ms - 120000, { kind: "user_message", lane: null, role: null, turn: "a3", text: "Every 15 minutes, tell me when a new release is out." }),
      item(ms - 110000, { turn: "a3", text: "I proposed watching `gh release list` every 15 minutes; it starts once you allow it below." }),
    ],
  };
  const crew = [
    { thread_id: "r1", name: "scout", role: "researcher", org_role: "researcher", state: "working", live: true, agent: "claude", provider: "anthropic", model: "haiku", doing: "Read city.kita.tokyo.jp", reported: false, started_at: now * 1000 - 120000, replies: 1, last_reply_at: now - 60, workspace_state: null, trial_state: null, needs: null, read_only: true },
    { thread_id: "r1", name: "bookkeeper", role: "engineer", org_role: "engineer", state: "done", live: false, agent: "codex", provider: "openai", model: "gpt-5.6", doing: null, reported: true, started_at: now * 1000 - 86400000, replies: 3, last_reply_at: now - 5400, workspace_state: null, trial_state: null, needs: null, read_only: false },
  ];
  const base = window.__TAURI__.core.invoke;
  window.__TAURI__.core.invoke = (name, args = {}) => {
    if (name === "thread" && args.id === "r1") return Promise.resolve(residentThread);
    if (name === "members" && args.thread === "r1") return Promise.resolve(crew);
    return name in answers ? Promise.resolve(answers[name]) : base(name, args);
  };
})();
