// The assistant's own pieces. An assistant is a conversation like any other — its channel, its
// members and their threads are the window's (`docs/resident-organization-design.md`) — so this
// file only holds what the conversation does not have: the card for a routine an agent proposed
// and the person allows, and the screen that makes the first assistant.
//
// A classic script like `routines.js`, inside a function so its names do not meet that file's
// in the page's one global scope; it reaches the window as `window.OrochiAssistant`.
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

  function proposalCard(routine, ctx) {
    const { t, call } = ctx;
    const card = node("div", "as-proposal");
    card.dataset.routine = routine.id;
    const { when, command } = proposalWords(routine);
    card.append(node("h4", "", command ? t("Proposed watch") : t("Proposed routine")));
    card.append(node("div", "as-when", words(when, ctx.lang)));
    if (command) card.append(node("code", "as-command", command));
    if (routine.member) card.append(node("div", "as-by", t("Run by {name}").replace("{name}", routine.member)));
    card.append(node("p", "as-what", routine.instruction));
    card.append(
      node(
        "p",
        "as-note",
        command ? t("This command runs here with nobody present. The agent it wakes reads, and asks you before it changes anything.") : t("Nobody is there when it runs: it reads, and asks you before it changes anything."),
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
      await ctx.openThread(made.thread);
    }, "primary");
    box.append(name, about, make);
    if (ctx.close) box.append(button("×", () => ctx.close(), "as-close"));
    page.append(box);
  }

  const api = { proposalCard, proposalWords, drawStart };
  if (typeof window !== "undefined") window.OrochiAssistant = api;
  if (typeof module !== "undefined") module.exports = api;
})();
