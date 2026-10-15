//! The lead as the conversation's manager (`docs/manager-design.md`): the plan it keeps, the
//! merge trials and applies it asks Orochi for, and the inbox it is reached at between turns.
//!
//! Everything here is a request the lead made through its mailbox tools and Orochi decided;
//! the lead never writes the user's tree, picks a model or passes a gate. What the trials and
//! the inbox produce goes back to the lead as a turn Orochi writes, held and coalesced the way
//! helpers' reports are, because every such turn is a resumed session and costs like one.
use super::background::{Env, HOLD};
use crate::{
    merge::{self, State, Trial},
    workspaces::Workspaces,
};
use anyhow::{Context, Result, bail, ensure};
use futures::{StreamExt, stream::FuturesUnordered};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    time::{Duration, Instant},
};

/// How long a helper's message to an idle lead waits for its siblings before it wakes the
/// lead: three lines written in a row cost one turn, not three.
pub const MAIL_HOLD: Duration = Duration::from_secs(60);
/// How many times one helper may wake the lead; past that its messages wait for the next turn.
pub const WAKES_PER_HELPER: usize = 3;
/// A plan as the lead wrote it, at most.
const PLAN_CHARS: usize = 16 * 1024;

/// The lead's plan: the coordinator's management state, kept on the thread.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Plan {
    pub plan: Vec<String>,
    pub decisions: Vec<String>,
    pub open_questions: Vec<String>,
    pub next_step: String,
}

impl Plan {
    fn render(&self) -> String {
        let mut out = String::new();
        let list = |out: &mut String, title: &str, items: &[String]| {
            if items.is_empty() {
                return;
            }
            out.push_str(title);
            out.push('\n');
            for (index, item) in items.iter().enumerate() {
                out.push_str(&format!("  {}. {item}\n", index + 1));
            }
        };
        list(&mut out, "Plan:", &self.plan);
        list(&mut out, "Decisions:", &self.decisions);
        list(&mut out, "Open questions:", &self.open_questions);
        if !self.next_step.is_empty() {
            out.push_str(&format!("Next step: {}\n", self.next_step));
        }
        out
    }
}

/// What a trial or apply came back with.
// A decision is small beside a trial; boxing the trials would only move the size elsewhere.
#[allow(clippy::large_enum_variant)]
pub enum Done {
    /// A trial, or a resolved trial checked again; `resolve` asks for a resolution seat if it
    /// still has conflicts.
    Trial { trial: Trial, resolve: bool },
    Applied {
        trial: Trial,
        result: Result<Vec<PathBuf>>,
        /// The approved trial, when the tree had moved and another was tried in its place.
        stale: Option<Trial>,
    },
    /// The user's answer to an apply question.
    Decision { trial: String, yes: bool },
}

type Run<'a> = Pin<Box<dyn Future<Output = Done> + 'a>>;

/// Whether an apply may go ahead, as the gate decided.
pub enum Gate {
    /// A clean trial whose real checks passed, under `manager.apply = "auto"`.
    Auto,
    /// The user decides; the lines say what the trial found.
    Ask(Vec<String>),
}

#[derive(Default)]
pub struct Manager<'a> {
    pub plan: Option<Plan>,
    /// Every trial of this conversation, oldest first.
    pub trials: Vec<Trial>,
    runs: FuturesUnordered<Run<'a>>,
    /// Workspaces with a trial under way.
    running: Vec<String>,
    /// Trial ids applying right now.
    applying: Vec<String>,
    /// Resolution helper name → the trial it resolves.
    resolving: BTreeMap<String, String>,
    /// What the trials and applies found, waiting to go back to the lead.
    pub reports: Vec<(String, Instant)>,
    /// Applies the user (or the gate) approved, waiting for the tree to be free.
    pub approved: VecDeque<String>,
    inbox: Option<crate::mailbox::Inbox>,
    /// Messages that reached the inbox, with when.
    pub mail: Vec<(crate::mailbox::Message, Instant)>,
    /// Messages past a helper's wake budget, handed over with the next turn whatever wakes it.
    later: Vec<crate::mailbox::Message>,
    woke: BTreeMap<String, usize>,
}

impl<'a> Manager<'a> {
    /// `/new`: the next conversation has its own plan, trials and inbox.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// A lead writes its plan as lists; one that wrote a numbered paragraph instead (a real
    /// Claude lead did, 2026-10-16) is read line by line rather than refused.
    pub fn set_plan(&mut self, arguments: &Value) -> Result<Plan> {
        ensure!(
            arguments.to_string().chars().count() <= PLAN_CHARS,
            "plan too long; keep it to the points"
        );
        let lines = |value: &Value| -> Vec<String> {
            match value {
                Value::Array(items) => items
                    .iter()
                    .map(|item| match item {
                        Value::String(text) => text.trim().to_owned(),
                        other => other.to_string(),
                    })
                    .filter(|text| !text.is_empty())
                    .collect(),
                Value::String(text) => text
                    .lines()
                    .map(|line| {
                        line.trim()
                            .trim_start_matches(|c: char| {
                                c.is_ascii_digit() || c == '.' || c == ')' || c == '-' || c == '*'
                            })
                            .trim()
                            .to_owned()
                    })
                    .filter(|line| !line.is_empty())
                    .collect(),
                _ => vec![],
            }
        };
        let plan = Plan {
            plan: lines(&arguments["plan"]),
            decisions: lines(&arguments["decisions"]),
            open_questions: lines(&arguments["open_questions"]),
            next_step: match &arguments["next_step"] {
                Value::String(text) => text.trim().to_owned(),
                Value::Array(_) => lines(&arguments["next_step"]).join("; "),
                _ => String::new(),
            },
        };
        self.plan = Some(plan.clone());
        Ok(plan)
    }

    /// Where the conversation stands, from Orochi's rows, for a session that cannot resume
    /// (`docs/manager-design.md` §4.2). `None` when there is nothing to say.
    pub fn standing(
        &self,
        helpers: &[(String, String, String)],
        root: &Path,
        directory: &Path,
    ) -> Option<String> {
        let mut out = String::new();
        if let Some(plan) = &self.plan {
            out.push_str(&plan.render());
        }
        let mut team: Vec<String> = helpers
            .iter()
            .map(|(name, title, state)| format!("  `{name}` ({title}) · {state}"))
            .collect();
        // Opening the manifest would create the session directory; only read one that is.
        if directory.join("workspaces.json").exists()
            && let Ok(work) = Workspaces::open(root, directory)
        {
            for workspace in &work.workspaces {
                if helpers.iter().any(|(name, ..)| *name == workspace.name) {
                    continue;
                }
                let applied = self
                    .trials
                    .iter()
                    .any(|t| t.workspace == workspace.name && t.state == State::Applied);
                team.push(format!(
                    "  `{}` · worktree {} · {}",
                    workspace.name,
                    workspace.root.display(),
                    if applied {
                        "applied"
                    } else {
                        "retained, not merged"
                    }
                ));
            }
        }
        if !team.is_empty() {
            out.push_str("Team and worktrees:\n");
            out.push_str(&team.join("\n"));
            out.push('\n');
        }
        let merges: Vec<String> = self
            .trials
            .iter()
            .map(|t| format!("  {}", summary(t)))
            .collect();
        if !merges.is_empty() {
            out.push_str("Merge trials:\n");
            out.push_str(&merges.join("\n"));
            out.push('\n');
        }
        (!out.is_empty()).then(|| {
            format!("Where this conversation stands (written by Orochi from its records):\n{out}")
        })
    }

    /// `list_work`: the team as JSON for the lead.
    pub fn listing(
        &self,
        helpers: Vec<Value>,
        root: &Path,
        directory: &Path,
        siblings: Vec<Value>,
    ) -> Value {
        let mut workspaces = vec![];
        if directory.join("workspaces.json").exists()
            && let Ok(work) = Workspaces::open(root, directory)
        {
            for workspace in &work.workspaces {
                let changed = git(&workspace.root, &["diff", "--name-only", &workspace.base])
                    .map(|out| out.lines().map(str::to_owned).collect::<Vec<_>>())
                    .unwrap_or_default();
                let untracked = git(
                    &workspace.root,
                    &["ls-files", "--others", "--exclude-standard"],
                )
                .map(|out| out.lines().map(str::to_owned).collect::<Vec<_>>())
                .unwrap_or_default();
                workspaces.push(json!({
                    "name": workspace.name,
                    "root": workspace.root,
                    "changed": changed,
                    "new": untracked,
                    "applied": self.trials.iter().any(|t| t.workspace == workspace.name && t.state == State::Applied),
                }));
            }
        }
        let trials: Vec<Value> = self
            .trials
            .iter()
            .map(|t| {
                json!({
                    "trial": t.id, "workspace": t.workspace, "n": t.n, "state": t.state,
                    "files": t.files, "overlap": t.overlap, "conflicts": t.conflicts,
                    "checks": t.checks.iter().map(|c| json!({"name": c.name, "passed": c.passed})).collect::<Vec<_>>(),
                    "error": t.error,
                })
            })
            .collect();
        json!({
            "plan": self.plan,
            "helpers": helpers,
            "workspaces": workspaces,
            "merges": trials,
            "other_conversations": siblings,
        })
    }

    /// Starts a trial of `workspace` against the tree at `env.root`.
    pub fn start_trial(&mut self, env: Env<'a>, workspace: &str, resolve: bool) -> Result<String> {
        let work = Workspaces::open(env.root, &env.workspace)?;
        let found = work
            .find(workspace)
            .with_context(|| format!("no worktree named `{workspace}` in this conversation"))?;
        ensure!(
            !self.running.iter().any(|w| w == workspace),
            "a trial of `{workspace}` is already running"
        );
        let n = self
            .trials
            .iter()
            .filter(|t| t.workspace == workspace)
            .count()
            + 1;
        let id_hint = format!("{workspace}-{n}");
        let (source, workspace_entry, directory) = (
            env.root.to_path_buf(),
            crate::workspaces::Workspace {
                name: found.name.clone(),
                root: found.root.clone(),
                base: found.base.clone(),
                title: found.title.clone(),
            },
            env.workspace.clone(),
        );
        self.running.push(workspace.to_owned());
        self.runs.push(Box::pin(async move {
            let name = workspace_entry.name.clone();
            let merged = tokio::task::spawn_blocking(move || {
                merge::trial(&source, &workspace_entry, &directory, n)
            })
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
            .and_then(|r| r);
            let trial = match merged {
                Ok(mut trial) => {
                    if trial.state == State::Clean {
                        trial.checks = crate::evaluator::evaluate_with(
                            &env.config.evaluator,
                            &trial.root,
                            false,
                        )
                        .await;
                    }
                    trial
                }
                Err(error) => Trial {
                    id: format!("{name}-{n}"),
                    workspace: name,
                    n,
                    state: State::Failed,
                    root: PathBuf::new(),
                    worktree: PathBuf::new(),
                    ours: String::new(),
                    theirs: String::new(),
                    base: String::new(),
                    merged: String::new(),
                    files: vec![],
                    overlap: vec![],
                    conflicts: vec![],
                    checks: vec![],
                    error: Some(first_line(&format!("{error:#}"))),
                },
            };
            Done::Trial { trial, resolve }
        }));
        Ok(id_hint)
    }

    /// A trial's outcome, or a resolved trial's: recorded, and reported to the lead. Returns
    /// the trial when a resolution seat should be started for it.
    pub fn finished(&mut self, trial: Trial, resolve: bool) -> Option<Trial> {
        self.running.retain(|w| *w != trial.workspace);
        let report = format!(
            "Merge trial of `{}` (trial {}) finished: {}",
            trial.workspace,
            trial.id,
            summary_tail(&trial)
        );
        self.reports.push((report, Instant::now()));
        match self.trials.iter_mut().find(|t| t.id == trial.id) {
            Some(existing) => *existing = trial.clone(),
            None => self.trials.push(trial.clone()),
        }
        (resolve && trial.state == State::Conflicts).then_some(trial)
    }

    /// A resolution seat was started for `trial` under `name`.
    pub fn resolving(&mut self, name: &str, trial: &str) {
        if let Some(t) = self.trials.iter_mut().find(|t| t.id == trial) {
            t.state = State::Resolving;
        }
        self.resolving.insert(name.to_owned(), trial.to_owned());
    }

    /// A helper ended; if it was resolving a trial, the trial is checked again.
    pub fn helper_ended(&mut self, name: &str, env: Env<'a>) {
        let Some(id) = self.resolving.remove(name) else {
            return;
        };
        let Some(mut trial) = self.trials.iter().find(|t| t.id == id).cloned() else {
            return;
        };
        self.running.push(trial.workspace.clone());
        self.runs.push(Box::pin(async move {
            let rechecked = tokio::task::spawn_blocking(move || {
                let outcome = merge::rechecked(&mut trial);
                (trial, outcome)
            })
            .await;
            let trial = match rechecked {
                Ok((mut trial, Ok(()))) => {
                    if trial.state == State::Clean {
                        trial.checks = crate::evaluator::evaluate_with(
                            &env.config.evaluator,
                            &trial.root,
                            false,
                        )
                        .await;
                    }
                    trial
                }
                Ok((mut trial, Err(error))) => {
                    trial.state = State::Failed;
                    trial.error = Some(first_line(&format!("{error:#}")));
                    trial
                }
                Err(error) => failed_trial(&format!("{error}")),
            };
            Done::Trial {
                trial,
                resolve: false,
            }
        }));
    }

    /// Whether `apply_merge` may go ahead for the trial, and how.
    pub fn gate(&self, trial: &str, auto: bool) -> Result<(Trial, Gate)> {
        let trial = self
            .trials
            .iter()
            .find(|t| t.id == trial || format!("{}-{}", t.workspace, t.n) == trial)
            .cloned()
            .with_context(|| format!("no merge trial `{trial}`; list_work shows them"))?;
        match trial.state {
            State::Clean => {}
            State::Applied => bail!("trial {} is already applied", trial.id),
            State::Applying => bail!("trial {} is being applied", trial.id),
            State::Conflicts => bail!(
                "trial {} has conflicts; resolve them first (merge_trial with resolve=true)",
                trial.id
            ),
            State::Resolving => bail!("trial {} is being resolved", trial.id),
            State::Stale => bail!("trial {} is stale; run merge_trial again", trial.id),
            State::Declined => bail!("the user declined trial {}", trial.id),
            State::Failed => bail!(
                "trial {} failed: {}",
                trial.id,
                trial.error.clone().unwrap_or_default()
            ),
        }
        ensure!(
            !self.applying.contains(&trial.id) && !self.approved.contains(&trial.id),
            "an apply of trial {} is already under way",
            trial.id
        );
        let verified = trial
            .checks
            .iter()
            .any(|c| c.passed && !c.name.starts_with("git_"))
            && trial.checks.iter().all(|c| c.passed);
        if auto && verified {
            return Ok((trial, Gate::Auto));
        }
        Ok((trial.clone(), Gate::Ask(describe(&trial))))
    }

    /// Waits for the terminal's answer to the apply question.
    pub fn await_decision(
        &mut self,
        trial: &str,
        receiver: tokio::sync::oneshot::Receiver<Option<String>>,
    ) {
        let trial = trial.to_owned();
        self.applying.push(trial.clone());
        self.runs.push(Box::pin(async move {
            let yes = receiver.await.ok().flatten().is_some();
            Done::Decision { trial, yes }
        }));
    }

    /// Waits for a client's answer to the apply question left as a `prompts` row.
    pub fn await_prompt(&mut self, trial: &str, store: crate::activity::Shared, prompt: String) {
        let trial = trial.to_owned();
        self.applying.push(trial.clone());
        self.runs.push(Box::pin(async move {
            loop {
                let answered = store.lock().expect("activity store").prompt_answer(&prompt);
                match answered {
                    Ok(Some(answer)) => {
                        return Done::Decision {
                            trial,
                            yes: answer.is_some(),
                        };
                    }
                    Ok(None) => tokio::time::sleep(Duration::from_millis(500)).await,
                    Err(_) => return Done::Decision { trial, yes: false },
                }
            }
        }));
    }

    /// The user said no: the trial stays, declined, and the lead hears so.
    pub fn declined(&mut self, trial: &str) {
        self.applying.retain(|id| id != trial);
        if let Some(t) = self.trials.iter_mut().find(|t| t.id == trial) {
            t.state = State::Declined;
            self.reports.push((
                format!(
                    "Apply of `{}` (trial {}) declined by the user.",
                    t.workspace, t.id
                ),
                Instant::now(),
            ));
        }
    }

    /// The user said yes: the apply waits for the tree to be free.
    pub fn approve(&mut self, trial: &str) {
        self.applying.retain(|id| id != trial);
        self.approved.push_back(trial.to_owned());
    }

    /// Writes an approved trial into the tree, once no turn holds it. A tree that moved since
    /// the trial is tried once more, on the tree as it is now, and only a clean result whose
    /// checks pass is written; anything else goes back to the lead as a trial of its own.
    pub fn run_apply(&mut self, env: Env<'a>, id: &str) {
        let Some(trial) = self.trials.iter().find(|t| t.id == id).cloned() else {
            return;
        };
        // The record says it is being applied; the copy handed to `apply` stays clean, which
        // is what `apply` requires of it.
        if let Some(t) = self.trials.iter_mut().find(|t| t.id == id) {
            t.state = State::Applying;
        }
        self.applying.push(id.to_owned());
        let (data, root, directory) = (
            env.data.to_path_buf(),
            env.root.to_path_buf(),
            env.workspace.clone(),
        );
        let repository = env
            .store
            .repository_id(env.root)
            .map_err(|e| format!("{e:#}"));
        let next = self
            .trials
            .iter()
            .filter(|t| t.workspace == trial.workspace)
            .count()
            + 1;
        self.runs.push(Box::pin(async move {
            let (trial, outcome) = write(
                trial,
                data.clone(),
                root.clone(),
                directory.clone(),
                repository.clone(),
            )
            .await;
            if trial.state != State::Stale {
                return Done::Applied {
                    trial,
                    result: outcome,
                    stale: None,
                };
            }
            // The tree moved: once more, on the tree as it is now.
            let stale = trial;
            let (source, dir, name) = (root.clone(), directory.clone(), stale.workspace.clone());
            let again = tokio::task::spawn_blocking(move || -> Result<Trial> {
                let work = Workspaces::open(&source, &dir)?;
                let workspace = work
                    .find(&name)
                    .with_context(|| format!("no worktree named `{name}`"))?;
                merge::trial(&source, workspace, &dir, next)
            })
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
            .and_then(|r| r);
            let mut again = match again {
                Ok(again) => again,
                Err(error) => {
                    return Done::Applied {
                        trial: failed_trial(&format!("{error:#}")),
                        result: Err(error),
                        stale: Some(stale),
                    };
                }
            };
            if again.state == State::Clean {
                again.checks =
                    crate::evaluator::evaluate_with(&env.config.evaluator, &again.root, false)
                        .await;
            }
            if again.state != State::Clean || again.checks.iter().any(|c| !c.passed) {
                let why = format!(
                    "tried again on the moved tree as trial {}: {}",
                    again.id,
                    summary_tail(&again)
                );
                return Done::Applied {
                    trial: again,
                    result: Err(anyhow::anyhow!(why)),
                    stale: Some(stale),
                };
            }
            let (again, outcome) = write(again, data, root, directory, repository).await;
            Done::Applied {
                trial: again,
                result: outcome,
                stale: Some(stale),
            }
        }));
    }

    /// An apply came back; `stale` is the trial that was approved when the tree had moved.
    pub fn applied(&mut self, trial: Trial, result: &Result<Vec<PathBuf>>, stale: Option<Trial>) {
        if let Some(stale) = stale {
            self.applying.retain(|id| *id != stale.id);
            match self.trials.iter_mut().find(|t| t.id == stale.id) {
                Some(existing) => *existing = stale,
                None => self.trials.push(stale),
            }
        }
        self.applying.retain(|id| *id != trial.id);
        let report = match result {
            Ok(files) => format!(
                "Applied `{}` (trial {}): {} written.",
                trial.workspace,
                trial.id,
                files_phrase(files.len())
            ),
            Err(error) => format!(
                "Apply of `{}` (trial {}) did not happen: {}",
                trial.workspace,
                trial.id,
                first_line(&format!("{error:#}"))
            ),
        };
        self.reports.push((report, Instant::now()));
        match self.trials.iter_mut().find(|t| t.id == trial.id) {
            Some(existing) => *existing = trial,
            None if !trial.id.is_empty() => self.trials.push(trial),
            None => {}
        }
    }

    pub fn is_busy(&self) -> bool {
        !self.running.is_empty() || !self.applying.is_empty()
    }

    /// The next finished trial or apply. Pending for ever when none runs, so a `select!` arm
    /// guards it with `is_busy()`.
    pub async fn next(&mut self) -> Done {
        match self.runs.next().await {
            Some(done) => done,
            None => std::future::pending().await,
        }
    }

    // --- the inbox ---

    /// Holds the lead's name in the room while it is between turns.
    pub fn hold_inbox(&mut self, name: &str, root: &Path) {
        if self.inbox.is_none() {
            self.inbox = crate::mailbox::Inbox::hold(name, root);
        }
    }
    /// Gives the name back, for the lead's next session.
    pub fn release_inbox(&mut self) {
        if let Some(inbox) = self.inbox.take() {
            for message in inbox.read() {
                self.mail.push((message, Instant::now()));
            }
        }
    }
    /// Reads what arrived for the lead; a sender past its wake budget is heard next turn.
    pub fn poll_inbox(&mut self) {
        let Some(inbox) = &self.inbox else { return };
        for message in inbox.read() {
            let wakes = self.woke.get(&message.from).copied().unwrap_or(0);
            if wakes >= WAKES_PER_HELPER {
                self.later.push(message);
            } else {
                self.mail.push((message, Instant::now()));
            }
        }
    }

    /// Whether what is waiting should go back to the lead now: a report once nothing else is
    /// still at work or it has waited `HOLD`, mail once it has waited `MAIL_HOLD`.
    pub fn due(&self, idle_team: bool) -> bool {
        let report_due = self
            .reports
            .iter()
            .map(|(_, at)| at.elapsed())
            .max()
            .is_some_and(|waited| idle_team || waited >= HOLD);
        let mail_due = self
            .mail
            .iter()
            .map(|(_, at)| at.elapsed())
            .max()
            .is_some_and(|waited| waited >= MAIL_HOLD);
        report_due || mail_due
    }
    pub fn has_pending(&self) -> bool {
        !self.reports.is_empty() || !self.mail.is_empty()
    }

    /// Takes everything waiting as the text of one turn, and which origin names it.
    pub fn take(&mut self) -> (Vec<String>, &'static str) {
        let mut parts: Vec<String> = self.reports.drain(..).map(|(text, _)| text).collect();
        let origin = if parts.is_empty() { "mail" } else { "merge" };
        for (message, _) in self.mail.drain(..) {
            *self.woke.entry(message.from.clone()).or_insert(0) += 1;
            parts.push(format!(
                "Message from `{}` while you were between turns:\n{}",
                message.from, message.body
            ));
        }
        for message in self.later.drain(..) {
            parts.push(format!(
                "Message from `{}`, held for this turn:\n{}",
                message.from, message.body
            ));
        }
        (parts, origin)
    }
}

/// The resolution seat's instructions, the integrator's `untangle` turn in another chair.
pub fn resolution_task(trial: &Trial) -> String {
    format!(
        "Resolve the merge conflicts in your worktree between the user's working tree and the worktree `{}`: {}. Keep what each side meant, remove every conflict marker and change nothing else. Run the project's checks if you can; the tree may not pass them yet. Report what you resolved and how.",
        trial.workspace,
        serde_json::to_string(&trial.conflicts).unwrap_or_default()
    )
}

/// What the apply question says about the trial.
fn describe(trial: &Trial) -> Vec<String> {
    let mut lines = vec![format!(
        "{} · {}",
        files_phrase(trial.files.len()),
        checks_phrase(&trial.checks)
    )];
    if !trial.overlap.is_empty() {
        lines.push(format!(
            "{} you also changed: {}",
            files_phrase(trial.overlap.len()),
            trial.overlap.join(", ")
        ));
    }
    lines
}

pub fn summary(trial: &Trial) -> String {
    format!(
        "`{}` trial {} · {}",
        trial.workspace,
        trial.id,
        summary_tail(trial)
    )
}

fn summary_tail(trial: &Trial) -> String {
    match trial.state {
        State::Clean => {
            let mut text = format!(
                "clean · {} · {}",
                files_phrase(trial.files.len()),
                checks_phrase(&trial.checks)
            );
            if !trial.overlap.is_empty() {
                text.push_str(&format!(
                    " · {} both sides changed ({})",
                    files_phrase(trial.overlap.len()),
                    trial.overlap.join(", ")
                ));
            }
            text.push_str(&format!(" · apply_merge(\"{}\") applies it", trial.id));
            text
        }
        State::Conflicts => format!(
            "{} ({}); merge_trial(\"{}\", resolve=true) resolves them",
            conflicts_phrase(trial.conflicts.len()),
            trial.conflicts.join(", "),
            trial.workspace
        ),
        State::Resolving => "being resolved".into(),
        State::Failed => format!("failed: {}", trial.error.clone().unwrap_or_default()),
        State::Applying => "being applied".into(),
        State::Applied => format!("applied · {} written", files_phrase(trial.files.len())),
        State::Declined => "declined by the user".into(),
        State::Stale => "stale; the tree moved".into(),
    }
}

fn files_phrase(n: usize) -> String {
    match n {
        1 => "1 file".into(),
        n => format!("{n} files"),
    }
}
fn conflicts_phrase(n: usize) -> String {
    match n {
        1 => "1 conflict".into(),
        n => format!("{n} conflicts"),
    }
}
fn checks_phrase(checks: &[crate::types::CheckResult]) -> String {
    if checks.is_empty() {
        return "no checks configured".into();
    }
    let failed: Vec<&str> = checks
        .iter()
        .filter(|c| !c.passed)
        .map(|c| c.name.as_str())
        .collect();
    if failed.is_empty() {
        format!(
            "checks passed ({})",
            checks
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else {
        format!("checks failed ({})", failed.join(", "))
    }
}

/// Writes one trial under the repository's exclusive lock, on a blocking thread.
async fn write(
    mut trial: Trial,
    data: PathBuf,
    root: PathBuf,
    directory: PathBuf,
    repository: std::result::Result<String, String>,
) -> (Trial, Result<Vec<PathBuf>>) {
    tokio::task::spawn_blocking(move || {
        let outcome = (|| -> Result<Vec<PathBuf>> {
            let repository = repository.map_err(|e| anyhow::anyhow!(e))?;
            let _lock = crate::storage::workspace_lock(&data, &repository, false)?;
            merge::apply(&root, &directory, &mut trial)
        })();
        (trial, outcome)
    })
    .await
    .unwrap_or_else(|error| {
        (
            failed_trial(&format!("{error}")),
            Err(anyhow::anyhow!("{error}")),
        )
    })
}

/// A trial that never got as far as a merge, carrying only why.
fn failed_trial(error: &str) -> Trial {
    Trial {
        id: String::new(),
        workspace: String::new(),
        n: 0,
        state: State::Failed,
        root: PathBuf::new(),
        worktree: PathBuf::new(),
        ours: String::new(),
        theirs: String::new(),
        base: String::new(),
        merged: String::new(),
        files: vec![],
        overlap: vec![],
        conflicts: vec![],
        checks: vec![],
        error: Some(first_line(error)),
    }
}

fn first_line(text: &str) -> String {
    text.lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(200)
        .collect()
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
