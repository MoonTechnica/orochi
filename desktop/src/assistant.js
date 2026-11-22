// The assistant: one conversation with a resident, drawn the way a messenger draws it
// (`docs/resident-agent-design.md` §23). What its routines found arrives as its messages, what
// it proposes waits as a card to allow or decline, and what it read for an answer is listed
// under the answer. The full conversation (tool calls, routes, checks) is one click away.
//
// A classic script like `routines.js`, inside a function so its names do not meet that file's
// in the page's one global scope; it reaches the window as `window.OrochiAssistant`, and the
// window gives it what it needs (`call`, `t`, `markdown`, `askCard`, how to open things).
(() => {
  const node = (tag, className, value) => {
    const n = document.createElement(tag);
    if (className) n.className = className;
    if (value !== undefined) n.textContent = value;
    return n;
  };
  const button = (label, onclick, className = "") => {
    const b = node("button", className, label);
    b.type = "button";
    b.addEventListener("click", onclick);
    return b;
  };

  const MAX_SOURCES = 4;
  const baseName = (path) => String(path).split(/[\\/]/).filter(Boolean).pop() || path;
  const host = (url) => {
    const m = /^https?:\/\/([^/?#]+)/i.exec(url);
    return m ? m[1].replace(/^www\./, "") : url;
  };

  /// What one tool call read or fetched, if it was reading or fetching.
  function sourcesOf(item) {
    if (item.kind !== "tool_call" || !item.data) return [];
    const d = item.data;
    const raw = d.raw_input || {};
    if (d.tool_kind === "fetch" || typeof raw.url === "string") {
      const url = typeof raw.url === "string" ? raw.url : "";
      return /^https?:\/\//i.test(url) ? [{ kind: "web", target: url, label: host(url) }] : [];
    }
    if (d.tool_kind !== "read") return [];
    const paths = Array.isArray(d.locations) ? d.locations.map((l) => String(l?.path || "")).filter(Boolean) : [];
    if (!paths.length && typeof raw.file_path === "string") paths.push(raw.file_path);
    return paths.map((p) => ({ kind: "file", target: p, label: baseName(p) }));
  }

  /// The routine a wake's text names: `…started by a routine, "Morning brief" (due …`.
  function wokenBy(text) {
    const m = /started by a routine, "([^"]+)"/.exec(text || "");
    return m ? m[1] : null;
  }

  /// A long reply shown folded: the first `limit` characters at a paragraph or sentence end.
  function folded(text, limit = 600) {
    if (text.length <= limit + 80) return { head: text, more: false };
    const cut = text.slice(0, limit);
    const at = Math.max(cut.lastIndexOf("\n"), cut.lastIndexOf("。"), cut.lastIndexOf(". "));
    return { head: `${(at > limit / 2 ? cut.slice(0, at + 1) : cut).trimEnd()}…`, more: true };
  }

  /// The conversation as lines: the person's, the assistant's (several streamed messages of one
  /// turn are one bubble, with what was read for it), a routine's wake, then what waits on the
  /// person — its questions and its proposals — and a typing bubble while it works.
  function chatLines(thread, routines = [], prompts = []) {
    const out = [];
    let read = [];
    let turn = null;
    const items = [...(thread?.items || [])]
      .filter((i) => !i.member && !(i.lane != null && i.lane > 0))
      .sort((a, b) => (a.at || 0) - (b.at || 0) || a.seq - b.seq);
    for (const item of items) {
      if (item.kind === "user_message") {
        read = [];
        turn = item.turn;
        const origin = item.turn_origin;
        if (origin === "routine") out.push({ type: "woke", key: `u${item.seq}`, routine: wokenBy(item.text), at: item.at });
        else if (!["background", "merge", "mail"].includes(origin)) out.push({ type: "me", key: `u${item.seq}`, text: item.text, at: item.at });
      } else if (item.kind === "tool_call") {
        for (const s of sourcesOf(item)) if (!read.some((r) => r.target === s.target)) read.push(s);
      } else if (item.kind === "agent_message" && item.text) {
        const last = out[out.length - 1];
        if (last?.type === "agent" && last.turn === item.turn) {
          last.text = `${last.text}\n\n${item.text}`;
          for (const s of read) if (!last.sources.some((r) => r.target === s.target)) last.sources.push(s);
          last.sources = last.sources.slice(0, MAX_SOURCES);
        } else {
          out.push({ type: "agent", key: `m${item.seq}`, text: item.text, at: item.at, turn: item.turn, sources: read.slice(0, MAX_SOURCES) });
        }
        read = [];
      }
    }
    const id = thread?.thread?.id;
    for (const p of prompts.filter((p) => p.thread_id === id)) out.push({ type: "ask", key: `ask${p.id}`, prompt: p });
    for (const r of routines.filter((r) => r.proposed && r.thread_id === id)) out.push({ type: "proposal", key: `pr${r.id}`, routine: r });
    if (thread?.thread?.status === "working") out.push({ type: "typing", key: "typing" });
    return out;
  }

  /// A proposal in words: when it would run, or the command it would watch and how often.
  function proposalWords(r) {
    if (r.watch) {
      const secs = r.every_secs || 0;
      const every = secs >= 3600 && secs % 3600 === 0 ? `${secs / 3600}h` : `${Math.round(secs / 60)}m`;
      return { when: `every ${every}`, command: r.watch };
    }
    return { when: r.schedule || "", command: null };
  }

  const words = (text, lang) =>
    (typeof window !== "undefined" && window.OrochiRoutines?.scheduleWords?.(text, lang)) || text;

  // The store keeps some times in seconds and the rest in milliseconds (`toMs` on the phone).
  const ms = (at) => (at < 100_000_000_000 ? at * 1000 : at);
  const clock = (at) => new Date(ms(at)).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  const dayOf = (at) => new Date(ms(at)).toDateString();

  // ---- the page ------------------------------------------------------------------------

  /// Which resident the page shows, while the window is open.
  let chosen = null;

  /// Draws the page whole: the head, the list and the composer. `update` redraws only the
  /// list, so what is being typed is never taken away by a refresh.
  async function drawAssistant(page, ctx) {
    const { call, t } = ctx;
    page.replaceChildren();
    const residents = (await call("residents", {})) || [];
    if (!residents.some((r) => r.thread_id === chosen)) chosen = residents[0]?.thread_id || null;
    if (!chosen) {
      drawStart(page, ctx);
      return;
    }
    const head = node("header", "as-head");
    const face = node("div", "as-face");
    const who = node("div", "as-who");
    const name = node("strong", "as-name");
    const status = node("span", "as-status");
    who.append(name, status);
    head.append(face, who);
    if (residents.length > 1) {
      const pick = node("select", "as-pick");
      pick.setAttribute("aria-label", t("Assistant"));
      for (const r of residents) {
        const o = node("option", "", r.name);
        o.value = r.thread_id;
        o.selected = r.thread_id === chosen;
        pick.append(o);
      }
      pick.addEventListener("change", () => {
        chosen = pick.value;
        drawAssistant(page, ctx);
      });
      head.append(pick);
    }
    const actions = node("div", "as-actions");
    // There is no list of routines: the person asks the assistant to stop one. This is the
    // one switch for when that is not enough, and it stops every routine of this resident.
    const pause = button(t("Pause"), async () => {
      const me = ((await call("residents", {})) || []).find((r) => r.thread_id === chosen);
      if (!me) return;
      const done = await call("resident_action", { name: me.name, action: me.paused ? "resume" : "pause" });
      if (done === null) ctx.report?.(t("That did not work."));
      await update(page, ctx);
    }, "as-pause");
    pause.title = t("Stops everything it has scheduled until you resume it");
    actions.append(pause, button(t("Full conversation"), () => ctx.openThread(chosen)));
    if (ctx.close) actions.append(button("×", () => ctx.close(), "as-close"));
    head.append(actions);
    const list = node("div", "as-list");
    list.setAttribute("role", "log");
    list.setAttribute("aria-label", t("Conversation"));
    const form = node("form", "as-composer");
    const input = node("textarea", "as-input");
    input.rows = 1;
    const send = node("button", "primary", t("Send"));
    send.type = "submit";
    form.append(input, send);
    const submit = async () => {
      const value = input.value.trim();
      if (!value || send.disabled) return;
      send.disabled = true;
      try {
        await call("send", { thread: chosen, text: value });
        await call("ensure_host", { thread: chosen });
        input.value = "";
      } finally {
        send.disabled = false;
      }
      await update(page, ctx);
    };
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      submit();
    });
    input.addEventListener("keydown", (event) => {
      // As the conversation's own composer: Cmd-Enter sends, Enter is a new line.
      if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
        event.preventDefault();
        submit();
      }
    });
    page.append(head, list, form);
    page.dataset.thread = chosen;
    await update(page, ctx);
  }

  /// Redraws what is said and the head's state; keeps the place in the list unless the reader
  /// was at the newest line.
  async function update(page, ctx) {
    const { call, t } = ctx;
    const list = page.querySelector?.(".as-list");
    if (!list || !chosen) return;
    const [thread, routines, prompts, residents] = await Promise.all([
      call("thread", { id: chosen }),
      call("routines", {}),
      call("open_prompts", {}),
      call("residents", {}),
    ]);
    const me = (residents || []).find((r) => r.thread_id === chosen);
    page.dataset.home = me?.home || thread?.thread?.cwd || "";
    const title = me?.name || thread?.thread?.title || t("Assistant");
    page.querySelector(".as-name").textContent = title;
    page.querySelector(".as-face").textContent = title.slice(0, 1).toUpperCase();
    const working = thread?.thread?.status === "working";
    page.querySelector(".as-face").classList.toggle("working", working);
    const pause = page.querySelector(".as-pause");
    if (pause) pause.textContent = me?.paused ? t("Resume") : t("Pause");
    const next = me?.next_at ? clock(me.next_at) : null;
    page.querySelector(".as-status").textContent = working
      ? t("Working…")
      : me?.paused
        ? t("paused")
        : next
          ? t("Next wakes at {when}").replace("{when}", next)
          : t("Nothing scheduled — ask it to keep you up with something");
    page.querySelector(".as-input").placeholder = t("Message {name}").replace("{name}", title);
    const atEnd = list.scrollHeight - list.scrollTop - list.clientHeight < 40;
    const lines = chatLines(thread, routines || [], prompts || []);
    list.replaceChildren();
    if (!lines.length) {
      list.append(node("p", "empty", t("Ask it anything, or for something every morning: it proposes, you allow.")));
    }
    let day = null;
    for (const line of lines) {
      if (line.at && dayOf(line.at) !== day) {
        day = dayOf(line.at);
        list.append(node("div", "as-day", new Date(ms(line.at)).toLocaleDateString(undefined, { month: "short", day: "numeric", weekday: "short" })));
      }
      list.append(drawLine(line, ctx));
    }
    if (atEnd || !list.dataset.drawn) list.scrollTop = list.scrollHeight;
    list.dataset.drawn = "1";
  }

  function drawLine(line, ctx) {
    const { t } = ctx;
    switch (line.type) {
      case "me": {
        const row = node("div", "as-row me");
        row.append(node("div", "as-bubble me", line.text));
        return row;
      }
      case "agent": {
        const row = node("div", "as-row agent");
        const bubble = node("div", "as-bubble agent");
        const { head, more } = folded(line.text);
        const body = node("div", "as-text");
        body.append(ctx.markdown(more ? head : line.text));
        bubble.append(body);
        if (more) {
          const toggle = button(t("Show more"), () => {
            const open = toggle.dataset.open !== "1";
            toggle.dataset.open = open ? "1" : "0";
            body.replaceChildren(ctx.markdown(open ? line.text : head));
            toggle.textContent = open ? t("Show less") : t("Show more");
          }, "link as-more");
          bubble.append(toggle);
        }
        if (line.sources.length) {
          const sources = node("div", "as-sources");
          for (const s of line.sources) {
            const link = button("", () => ctx.openSource(s), "link as-source");
            if (ctx.drawn) link.append(ctx.drawn(s.kind === "web" ? "globe" : "file"));
            link.append(node("span", "", s.label));
            link.dataset.kind = s.kind;
            link.title = s.target;
            sources.append(link);
          }
          bubble.append(sources);
        }
        row.append(bubble);
        return row;
      }
      case "woke": {
        const at = line.at ? ` · ${clock(line.at)}` : "";
        return node(
          "div",
          "as-woke",
          (line.routine ? t("Woke for “{name}”").replace("{name}", line.routine) : t("Woke by a routine")) + at,
        );
      }
      case "typing": {
        const row = node("div", "as-row agent");
        row.append(node("div", "as-bubble agent as-typing", "•••"));
        return row;
      }
      // What waits on the person sits on the assistant's side, like what it says.
      case "ask":
      case "proposal": {
        const row = node("div", "as-row agent");
        row.append(line.type === "ask" ? ctx.askCard(line.prompt) : proposalCard(line.routine, ctx));
        return row;
      }
      default:
        return node("div");
    }
  }

  /// What the assistant proposed: when it would run (or the command it would run with nobody
  /// present, shown whole), what it would do, and the person's answer. Nothing runs before Allow.
  function proposalCard(routine, ctx) {
    const { t, call } = ctx;
    const card = node("div", "as-proposal");
    card.dataset.routine = routine.id;
    const { when, command } = proposalWords(routine);
    card.append(node("h4", "", command ? t("Proposed watch") : t("Proposed routine")));
    card.append(node("div", "as-when", words(when, ctx.lang)));
    if (command) card.append(node("code", "as-command", command));
    card.append(node("p", "as-what", routine.instruction));
    card.append(
      node(
        "p",
        "as-note",
        command ? t("It runs this command here with nobody present, reading only.") : t("It runs reading only, with nobody present."),
      ),
    );
    const row = node("div", "as-answer");
    const decide = (action) => async () => {
      for (const b of row.querySelectorAll?.("button") || []) b.disabled = true;
      const done = await call("routine_action", { id: routine.id, action });
      if (done === null) ctx.report?.(t("That did not work."));
      await ctx.refresh();
    };
    row.append(button(t("Don't allow"), decide("decline")), button(t("Allow"), decide("approve"), "primary"));
    card.append(row);
    return card;
  }

  /// No resident yet: one is made here, with a name and who it is.
  function drawStart(page, ctx) {
    const { t, call } = ctx;
    const box = node("div", "as-start");
    box.append(node("h2", "", t("Your assistant")));
    box.append(
      node(
        "p",
        "",
        t("An assistant lives on your desktop with a folder and notes of its own. Talk to it here; ask it to check something every morning and it proposes a routine for you to allow."),
      ),
    );
    const name = node("input");
    name.value = "assistant";
    name.setAttribute("aria-label", t("Name"));
    name.addEventListener("input", () => {
      name.value = name.value.toLowerCase().replace(/[^a-z0-9-]/g, "");
    });
    const about = node("textarea");
    about.rows = 3;
    about.placeholder = t("Who is it, and what is it for?");
    about.setAttribute("aria-label", t("Who it is"));
    const make = button(t("Make my assistant"), async () => {
      if (!name.value) return;
      make.disabled = true;
      const made = await call("resident_add", { name: name.value, instruction: about.value });
      make.disabled = false;
      if (!made) return ctx.report?.(t("The resident was not made."));
      chosen = made.thread;
      await drawAssistant(page, ctx);
    }, "primary");
    box.append(name, about, make);
    if (ctx.close) box.append(button("×", () => ctx.close(), "as-close"));
    page.append(box);
  }

  const api = { chatLines, sourcesOf, wokenBy, folded, proposalWords, drawAssistant, update };
  if (typeof window !== "undefined") window.OrochiAssistant = api;
  if (typeof module !== "undefined") module.exports = api;
})();
