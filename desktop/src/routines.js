// The two settings pages for what wakes by itself (`docs/resident-agent-design.md` §14), and
// how a schedule is said. There is no page listing routines: the person asks the assistant
// (`assistant.js`) to start or stop one, as in a messenger, and every control here is the same
// library call `orochi routines` / `orochi notify` makes.
//
// A classic script, not a module: the window's own script is evaluated as a function body by
// its tests, so it cannot import, and this reaches it as `window.OrochiRoutines`.
//
// The window gives it what it needs (`call`, `t`, `report`, how to open a thread and the
// projects), so this file reads no globals and a test can run it against a stub.

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

/// A moment, as a list says it: today's clock, or the day and the clock.
function when(seconds, now = Date.now()) {
  if (!seconds) return "—";
  const date = new Date(seconds * 1000);
  const sameDay = date.toDateString() === new Date(now).toDateString();
  const clock = date.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  return sameDay
    ? clock
    : `${date.toLocaleDateString(undefined, { month: "short", day: "numeric" })} ${clock}`;
}


/// How far today's spending is toward the budget, as 0..1.
function share(spent, limit) {
  return limit > 0 ? Math.max(0, Math.min(1, spent / limit)) : 0;
}

const KINDS = {
  ask: "Needs you",
  report: "Report",
  mention: "Mentioned you",
  trouble: "Problem",
  scheduled: "Scheduled",
  budget: "Budget",
  keeper: "Keeper",
};

const SINKS = {
  sent: "sent",
  off: "off",
  held: "held for quiet hours",
  skipped_window: "window was open",
  coalesced: "joined an earlier push",
  not_registered: "phone not registered",
  no_key: "no key",
  no_phones: "no phone paired",
  failed: "failed",
};

function sink(status, t = (w) => w) {
  return SINKS[status] ? t(SINKS[status]) : status || "—";
}

// ---- settings ---------------------------------------------------------------------------

const row = (label, control, hint) => {
  const r = node("label", "rt-setting");
  r.append(node("span", "rt-setting-label", label), control);
  if (hint) r.append(node("span", "rt-sub", hint));
  return r;
};
const number = (value, min, max) => {
  const i = node("input");
  i.type = "number";
  i.value = String(value);
  i.min = String(min);
  i.max = String(max);
  return i;
};
const toggle = (checked) => {
  const i = node("input");
  i.type = "checkbox";
  i.checked = !!checked;
  return i;
};

/// Settings → Residents and routines.
async function drawRoutineSettings(host, ctx) {
  const { call, t, report } = ctx;
  host.replaceChildren();
  const [keeper, settings] = await Promise.all([call("keeper", {}), call("notify_settings", {})]);
  if (!settings) return host.append(node("p", "empty", t("The settings could not be read.")));
  const cfg = settings.routines;
  host.append(node("h3", "", t("Keeper")));
  const status = node("p", "", keeper?.running ? `${t("Running")} (pid ${keeper.status?.pid})` : t("Stopped. It starts when a routine is waiting and Orochi is used."));
  host.append(status);
  if (keeper) {
    const spend = node("p", "rt-spend");
    spend.append(
      node(
        "span",
        "rt-spend-label",
        `${t("Today")}: ${Math.round(keeper.tokens_today).toLocaleString()} / ${Math.round(keeper.daily_tokens).toLocaleString()} ${t("tokens")}`,
      ),
    );
    const meter = node("span", share(keeper.tokens_today, keeper.daily_tokens) >= 0.8 ? "rt-meter low" : "rt-meter");
    const fill = node("i");
    fill.style.width = `${Math.round(share(keeper.tokens_today, keeper.daily_tokens) * 100)}%`;
    meter.append(fill);
    spend.append(meter);
    host.append(spend);
  }
  if (keeper?.login_supported) {
    const login = toggle(keeper.login);
    login.addEventListener("change", async () => {
      const got = await call("keeper_login", { on: login.checked });
      if (!got) login.checked = !login.checked;
    });
    host.append(
      row(
        t("Start at login"),
        login,
        t("macOS will start Orochi's keeper when you log in, so routines run without the app open. Nothing outside your user's Library is changed."),
      ),
    );
  }
  host.append(node("h3", "", t("Routines")));
  const enabled = toggle(cfg.enabled);
  const daily = number(cfg.daily_tokens, 1000, 100000000);
  const interval = number(Math.round(cfg.min_interval_secs / 60), 1, 1440);
  const ask = number(Math.round(cfg.ask_secs / 60), 1, 1440);
  const late = number(Math.round(cfg.max_late_secs / 3600), 0, 168);
  const made = number(cfg.max_proposals, 0, cfg.max_per_thread);
  const expiry = number(cfg.proposal_days, 1, 365);
  host.append(
    row(t("Routines are on"), enabled),
    row(t("Tokens per day"), daily, t("Weighted tokens routines may spend in a day; the rest are skipped until tomorrow.")),
    row(t("Shortest time between runs (minutes)"), interval),
    row(t("Decline an unanswered question after (minutes)"), ask),
    row(t("Run one the Mac slept through if within (hours)"), late),
    row(t("Proposals an agent may leave waiting per conversation"), made),
    row(t("Days an unanswered proposal waits"), expiry),
  );
  host.append(
    button(t("Save"), async () => {
      const next = {
        ...cfg,
        enabled: enabled.checked,
        daily_tokens: Number(daily.value),
        min_interval_secs: Number(interval.value) * 60,
        ask_secs: Number(ask.value) * 60,
        max_late_secs: Number(late.value) * 3600,
        max_proposals: Number(made.value),
        proposal_days: Number(expiry.value),
      };
      const done = await call("save_section", { section: "routines", value: next });
      if (done !== null) report(t("Saved."));
    }, "primary"),
  );
}

/// Settings → Notifications, below the sound toggle the window already had.
async function drawNotifySettings(host, ctx) {
  const { call, t, report } = ctx;
  const settings = await call("notify_settings", {});
  if (!settings) return;
  const n = settings.notify;
  host.append(node("h3", "", t("Where it reaches you")));
  const mac = toggle(n.mac);
  host.append(row(t("A notification on this Mac"), mac, t("Used when no Orochi window is open to show it.")));

  host.append(node("h3", "", t("On your phone (OneSignal)")));
  const on = toggle(n.onesignal.enabled);
  const appId = node("input");
  appId.type = "text";
  appId.value = n.onesignal.app_id;
  appId.spellcheck = false;
  const key = node("input");
  key.type = "password";
  key.autocomplete = "off";
  key.placeholder = settings.key ? t("set — type to replace") : t("REST API key");
  host.append(
    row(t("Push to my phone"), on, t("OneSignal, and Apple or Google, carry what you let them see below.")),
    row(t("OneSignal app ID"), appId),
    row(t("REST API key"), key, settings.key ? `${t("Held in")} ${settings.key}` : t("Not set. It is kept in the keychain, never in the settings file.")),
  );
  if (settings.key) {
    host.append(
      button(t("Remove the key"), async () => {
        await call("notify_key", { action: "forget" });
        drawNotifyAgain(host, ctx);
      }),
    );
  }
  const level = node("select");
  for (const [v, l] of [["title", "Only the name of the routine"], ["summary", "The name and the first lines"], ["full", "Everything it says"]]) {
    const o = node("option", "", t(l));
    o.value = v;
    level.append(o);
  }
  level.value = n.level;
  const away = number(Math.round(n.away_secs / 60), 0, 1440);
  const quiet = node("input");
  quiet.type = "text";
  quiet.value = n.quiet;
  quiet.placeholder = "23:00-07:00";
  const wake = toggle(n.wake_for_approvals);
  host.append(
    row(t("What OneSignal is shown"), level),
    row(t("Only after I have been away (minutes)"), away),
    row(t("Quiet hours"), quiet, t("Nothing is pushed then; it is sent when they end.")),
    row(t("Wake me for approvals"), wake),
  );
  host.append(node("h3", "", t("What to tell me")));
  const kinds = new Map();
  for (const kind of Object.keys(KINDS)) {
    const box = toggle(n.kinds.includes(kind));
    kinds.set(kind, box);
    host.append(row(t(KINDS[kind]), box));
  }
  const save = button(t("Save"), async () => {
    const next = {
      ...n,
      mac: mac.checked,
      level: level.value,
      away_secs: Number(away.value) * 60,
      quiet: quiet.value.trim(),
      wake_for_approvals: wake.checked,
      kinds: [...kinds].filter(([, box]) => box.checked).map(([k]) => k),
      onesignal: { ...n.onesignal, enabled: on.checked, app_id: appId.value.trim() },
    };
    if (key.value.trim()) {
      const done = await call("notify_key", { action: "set", key: key.value.trim() });
      if (done === null) return;
      key.value = "";
    }
    const done = await call("save_section", { section: "notify", value: next });
    if (done !== null) {
      report(t("Saved."));
      drawNotifyAgain(host, ctx);
    }
  }, "primary");
  const test = button(t("Send a test"), async () => {
    const got = await call("notify_test", {});
    if (got) report(`Mac: ${sink(got.mac, t)} · ${t("phone")}: ${sink(got.phone, t)}`);
  });
  host.append(save, test);

  host.append(node("h3", "", t("Paired phones")));
  if (!settings.phones.length) {
    host.append(node("p", "empty", t("None. The Orochi phone app registers itself when it pairs and notifications are allowed.")));
  }
  for (const p of settings.phones) {
    const line = node("div", "rt-firing");
    line.append(node("span", "", `${p.name} · ${p.id.slice(0, 8)}…`));
    line.append(
      button(t("Remove"), async () => {
        await call("phone_forget", { id: p.id });
        drawNotifyAgain(host, ctx);
      }, "link"),
    );
    host.append(line);
  }
}

const DAYS_JA = { mon: "月", tue: "火", wed: "水", thu: "木", fri: "金", sat: "土", sun: "日" };
const UNIT_JA = { m: "分", h: "時間", d: "日" };

/// A schedule as the person reads it. The grammar is Orochi's English (`daily at 08:00`); in
/// Japanese it is said in Japanese, and anything not recognised (a cron line) is shown as written.
/// The same rules as the phone's `scheduleWords` in `@orochi/core`.
function scheduleWords(text, lang) {
  const s = String(text ?? "").trim();
  if (!String(lang || "").startsWith("ja") || !s) return s;
  const times = (list) => list.split(/\s*,\s*/).join("・");
  const unit = (u) => UNIT_JA[u.toLowerCase()];
  let m;
  if ((m = /^daily at (.+)$/i.exec(s))) return `毎日 ${times(m[1])}`;
  if ((m = /^weekdays at (.+)$/i.exec(s))) return `平日 ${times(m[1])}`;
  if ((m = /^weekends at (.+)$/i.exec(s))) return `週末 ${times(m[1])}`;
  if ((m = /^every ((?:mon|tue|wed|thu|fri|sat|sun)(?:\s*,\s*(?:mon|tue|wed|thu|fri|sat|sun))*) at (.+)$/i.exec(s)))
    return `毎週 ${m[1].split(/\s*,\s*/).map((d) => DAYS_JA[d.toLowerCase()] || d).join("・")} ${times(m[2])}`;
  if ((m = /^every (\d+)\s*([mhd]) from (\S+) to (\S+)$/i.exec(s))) return `${m[3]}〜${m[4]} の間 ${m[1]}${unit(m[2])}ごと`;
  if ((m = /^every (\d+)\s*([mhd])$/i.exec(s))) return `${m[1]}${unit(m[2])}ごと`;
  if ((m = /^in (\d+)\s*([mhd])$/i.exec(s))) return `${m[1]}${unit(m[2])}後に 1 回`;
  if ((m = /^on (\d{4}-\d{2}-\d{2}) at (\S+)$/i.exec(s))) return `${m[1]} ${m[2]} に 1 回`;
  if ((m = /^at (\S+)$/i.exec(s))) return `${m[1]} に 1 回`;
  return s;
}

// The page redraws itself after a change, keeping what comes before it (the sound toggle).
function drawNotifyAgain(host, ctx) {
  const mine = [...host.querySelectorAll("h3")].find((h) => h.textContent === ctx.t("Where it reaches you"));
  if (!mine) return;
  while (mine.nextSibling) mine.nextSibling.remove();
  mine.remove();
  drawNotifySettings(host, ctx);
}

const api = { when, share, sink, scheduleWords, drawRoutineSettings, drawNotifySettings };
if (typeof window !== "undefined") window.OrochiRoutines = api;
if (typeof module !== "undefined") module.exports = api;
