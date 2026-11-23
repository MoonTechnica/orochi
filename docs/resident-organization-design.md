# The Resident Organization: Design

Written: 2026-11-23. **Status: design only. Nothing in §4 onward is built.** §2 was read from the
code and the existing designs (`resident-agent-design.md`, `org-design.md`,
`team-channel-design.md`) and is cited; §3 is what the user asked for; §4 onward are proposals the
user has not yet confirmed, marked as such.

## 1. The goal, in the user's words (2026-11-23)

An assistant is **an AI agent team that works around the clock**: "a virtual organization of
one's own made of AI". Using Orochi's role assignment, the lead **makes the members the work
needs, assigns them tasks, and releases them when they are no longer used, and repeats this**. The
person talks with **every** member, and the members talk with **each other** while they do their
own tasks. A routine is run by **a member suitably assigned to it**, not by the lead alone.

This replaces two readings that were wrong, both mine, and both rejected by the user the same day:
a single voice with a hidden team behind it (Dot's shape), and one shared channel with everyone in
it (a reading of `team-channel-design.md` alone). Orca Bots is the product the user pointed to
([article](https://zenn.dev/kg_motors_mibot/articles/orca-bot-intro),
[orca.dev/docs/bots](https://orca.dev/docs/bots)): each bot has a name, a picture, a purpose, a
memory in Markdown (`Soul`) and automations, **workers** run its coding jobs in worktrees and show
under it, a bot may propose another bot and nothing exists until the person presses Create, the
person reaches any bot from the sidebar or with `@`, permissions are per bot (*Ask for approval*,
*Accept edits*, *Approve for me*, *Full access*), and the phone has a "Needs you" list. What those
pages do **not** say is how several bots split one job or talk to each other; this design does not
take that from Orca.

### The principle: do what a human organization does

The user's words (2026-11-23): human organizations already work like this, and the aim is to
realize the same thing with AI. So a question this design cannot settle from the code is settled by
asking what a company does.

| In a human organization | Here |
|---|---|
| A manager hires someone for a role the work needs | The lead hires a member by role (`start_agent`, `org.hire`) |
| A person has a job title, a duty, and a manager | A member has a role, a one-sentence duty and a manager (D4) |
| Work is assigned to the person it suits | A routine names its member (D6) |
| People talk to their manager, to colleagues and to the owner directly | The person reaches every member; members message each other (D7, D8) |
| A person remembers what they learned, in their own notes | A member's notes file (D3) |
| Nobody works every minute; they are reachable, and work when there is work | A member runs when something wakes it, and is always reachable (D2) |
| Each person has authority for their own job, within their manager's | Authority per member, never above the manager's (D9) |
| A person who is no longer needed is let go, and can be re-hired | Release keeps notes and session, so re-hiring continues them (D5) |
| The owner approves hiring and spending | The person allows a standing member and a routine; budgets bound every wake (D12, §5) |

## 2. Where Orochi stands

| Fact | Where |
|---|---|
| A resident is a name, a home folder, an instruction, an access value, and one thread; a routine belongs to a thread, so to a resident | `src/activity/residents.rs`, `routines.rs` |
| A role is data (`name`, `instruction`, `writes`, `lead`, `may_hire`); `researcher`, `designer`, `engineer`, `reviewer` ship; a lead hires one with `start_agent`, and `org.hire` checks headcount, depth and the refusal's sentence | `src/org.rs` |
| A member is a name within a conversation; the lead's assignment is a `messages` row, what is said about that work is the member's thread; `@name`, `@you`, `@channel` are read from the text against names that exist | `team-channel-design.md`, `src/mailbox.rs` |
| `start_agent` with an ended member's name **continues** it (its session by `session/load`, or its own thread as bounded text); a running member's name is refused; a note to an ended member does the same | `src/chat/mod.rs` `grant`, `Session::continue_mentioned` |
| A member is a row (`v_members`): name, role, state, whether it writes, what it is doing, replies | `src/activity.rs` |
| Helpers belong to `chat::Session`, not to a turn; a host leaves after `activity.host_idle_secs` with nothing to do | `src/chat/background.rs`, `src/chat/host.rs` |
| A routine's turn runs one team in one turn; a helper's answer comes back as a later turn of the same firing, held by `firings.waiting` | `resident-agent-design.md` §23 |
| What a routine's turn may change is `residents.access` (`read` default, `home`, `full`) and `routines.approval` (`ask`, `smart`, `off`) — **one setting for all residents** | `src/config.rs` (2026-11-23) |
| The desktop's assistant page and the phone's assistant tab show the lead's own messages and **hide members' items** | `desktop/src/assistant.js` (`!i.member`) |

## 3. What is missing

1. **A member does not outlive its lead's session.** There is no process that is up around the
   clock, and none should be (§4 D2). Today a member exists while a turn runs, and is continued
   only by a name said to it.
2. **No one decides to keep or release a member.** `stop_agent` exists; nothing says when a member
   that has no more work should go, or lets the lead keep one for the next wake.
3. **A routine has no executor.** It is run by the resident's lead; the lead may start helpers
   during the turn, but which member a routine *belongs to* is not recorded.
4. **The person cannot see or address the organization from the assistant.** Members' messages are
   hidden, and there is no list of who is on the team.
5. **Authority is one setting for everybody.** The user's rule (as in Orca) is per bot.

## 4. Decisions proposed (not yet confirmed)

| # | Question | Proposal | Why |
|---|---|---|---|
| D1 | What an assistant is | The resident is the **root lead of an organization**; its thread is the organization's channel. Members are the organization's, and are kept in the thread | One mechanism: `org.rs` and the team channel already describe it |
| D2 | "Around the clock" | **A member runs only when something wakes it** — a routine assigned to it, a message to it (from the person or another member), or its manager. No process idles. A member's *existence* (name, role, memory, session) is a row that lasts; its *running* is a turn | A resident that never sleeps would cost like a session every minute; the keeper already wakes by clock and watch, and the mail tick already wakes by message |
| D3 | A member's memory | Each member has a notes file in its own folder beside the lead's `NOTES.md`, written by it and editable by the person, put in front of it at every wake (as §12 of `resident-agent-design.md` does for a resident) | Orca's `Soul` and memory are plain Markdown the person can read and edit; a member that remembers nothing between wakes cannot be a colleague |
| D4 | Hiring | The lead hires by role with `start_agent`, as now; a hire that is to **last** past the turn is recorded as a member of the organization with its role, its manager and its duty (one sentence). The cap rule stays (`org.max_members`, depth, reports) | R3–R5 of `org-design.md` |
| D5 | Releasing | A member is **released** by its manager, by the person, or by a rule: nothing assigned and no wake for `org.release_after_days`. Release ends the member and keeps its notes and its session reference, so hiring the same name again continues it. A release the lead asks for is a request Orochi decides, like a hire | The user's loop: make members, use them, release them, repeat. Keeping notes makes re-hiring cheap |
| D6 | A routine's executor | **A routine names its member** (`routines.member`, a member name; none = the lead). The lead picks the member when it proposes the routine (hiring one for the duty if none fits), the proposal card says who, and the person may change it. When the routine is due the keeper wakes **that member** as the executor of the firing; the member's manager is its lead, and the report is made in the member's name | The user's instruction: assign the routine to the member that suits it |
| D7 | The person and every member | The person writes to any member: `@name` in the assistant's conversation, or its own thread (`team-channel-design.md`). A member's reply shows under its **own name and role**, never as the lead's | The user's instruction: talk with all the subordinates |
| D8 | Members and each other | Members message each other through the mailbox and the channel as the existing design has it, so a member waits on, asks and answers a colleague while it works; what the person sees is the channel, with each post under its author | The user's instruction: the team communicates among itself while doing its own tasks |
| D9 | Authority is per member | `access` and `approval` move from `[residents]` / `[routines]` to the **member**: the person sets the resident's ceiling, a role sets what a member may do within it, and a member never exceeds its manager. Default stays `read` | Orca's permissions are per bot; the global setting built on 2026-11-23 is the interim |
| D10 | Writing without a worktree | As built: a helper on a routine's turn works in its lead's folder when access allows. A member with a folder of its own (D3) writes there first | The user's rule: the assistant must be able to write without a repository |
| D11 | Where the person sees it | **The assistant's page is the organization**: the members (name, role, picture, state, what each is doing and its duty), who is assigned which routine, a "needs you" list across all of them, and the conversation with each. Same on the phone | Orca's sidebar of bots, with workers under each |
| D12 | Proposing a member | The lead may propose a **standing** member the way it proposes a routine: nothing exists until the person allows it, shown with the role and the duty | Orca: a bot may propose another bot and nothing is created until the person presses Create |

## 5. Invariants to keep

- A member's running is always caused by something: a routine's due time, a message, its manager.
  Nothing idles to look busy.
- Every refusal is a sentence naming the setting that decided it (`org-design.md` §7).
- Nothing a member changes reaches the person's own tree except by the approval its authority says.
- The cost gates apply to every wake: the routine budget, `worth_seating`, the daily tokens.
- A message from a member is stored and shown under its author; no member speaks as another.

## 6. Build order

| Step | Content |
|---|---|
| S1 | (built, 2026-11-23: the assistant is the window's and the phone's own conversation, with its members, threads and the Office's characters) Show the organization: members, roles, state, duty, and each member's conversation, on the desktop and the phone (data exists in `v_members`; the assistant page stops hiding members) |
| S2 | `@name` in the assistant's conversation reaches that member (continues it) |
| S3 | `routines.member`: the lead names the executor when proposing; the card says who; **built (2026-11-23)**. The routine's turn is still the lead's, and its brief tells the lead to hand the routine to that member with `start_agent` (continuing it if it is on the team) and not do it itself; the member's answer is a follow-up of the same routine, and the report is made then. The keeper does not wake a member without the lead. Tested (`a_routine_given_to_a_member_tells_the_lead_to_hand_it_to_them`, mock agent) and run once with a real agent (Claude haiku, isolated data directory: the lead proposed a routine with `member = scout`, was allowed, and on its run handed it to `scout`, which reported under its own name before the lead's report) |
| S4 | Standing members: a hire that lasts, its notes file, release (by the lead's request, the person, and the idle rule). **Built in part (2026-11-23):** a member is a row that lasts and is continued by its name, and a resident's wake is told who is on the team (`resident::team_of`) so the lead uses a member again or releases one with `stop_agent`. **Not built:** a member's own notes file, the idle rule that releases by itself, and the person releasing one from a screen |
| S5 | Authority per member (D9), replacing the global settings. **Built in part (2026-11-23):** the setting is per *resident* (`residents.reach`: `orochi resident reach <name> read\|home\|full\|default`, default `residents.access`), which is Orca's per-bot setting; a member never writes where the resident may not. `routines.approval` is still one setting for all, and neither is in a window or phone screen yet |
| S6 | The lead proposes a standing member; the person allows it |
| S7 | A run with a real agent, recorded in a dated `real-validation-*.md` |

## 7. Not decided

- Whether a member has an agent or model of its own (today the scorer chooses, as for any helper).
- How long an unused member's notes and session are kept after it is released.
- Whether a member may be woken by something outside Orochi (GitHub, Slack, mail), as Orca's bots
  are; the keeper's watches are the only such trigger today.
- A budget per member; the routine budget is the only one now.
- Pictures for members: where they come from (a choice of the person, or a generated one).

## 8. What is verified

Nothing in §4 has been built or run. §2's last two rows describe the state after the work of
2026-11-23 (`residents.access`, `routines.approval`, a helper writing in its lead's folder), which
is covered by automated tests against the mock agent only, with no real agent.
