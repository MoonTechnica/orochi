//! Durable role handoffs. A logical role can be held by successive independent ACP sessions.
mod apply;
pub mod graph;
pub mod plan;
mod turn;
mod workspace;

pub use graph::{Part, Unit};
pub use turn::{Coordination, Outbound, TurnKind};
pub use workspace::{Conflict, copy_workspace};

use crate::{
    agents::{AgentError, ErrorKind},
    config::Config,
    policy::Registry,
    storage::Store,
    types::*,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::{Component, Path, PathBuf},
};

fn yes() -> bool {
    true
}
fn default_rounds() -> usize {
    2
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Participant {
    pub id: String,
    pub role: Role,
    /// Preferred agent; empty means automatic selection. Use fallback=false to pin it.
    #[serde(default)]
    pub agent: String,
    pub model: Option<String>,
    pub reasoning: Option<String>,
    pub mode: Option<String>,
    #[serde(default = "yes")]
    pub fallback: bool,
    /// Empty permits all enabled agents in the user's trusted configuration.
    #[serde(default)]
    pub allowed_agents: Vec<String>,
    /// Implementers only: relative path prefixes this participant is asked to own.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Coordinator,
    Implementer,
    Reviewer,
    Integrator,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Discussion {
    /// Rounds in which participants answer messages addressed to them.
    #[serde(default = "default_rounds")]
    pub max_rounds: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub participants: Vec<Participant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discussion: Option<Discussion>,
    /// The work in order. Empty: every implementer takes the whole task at once, as before
    /// parts existed. Otherwise implementers are seats the parts run on, as many at once as
    /// there are implementers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<Part>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    /// Participant indices that work concurrently in separate workspaces.
    Turn(Vec<usize>),
    Merge,
    Discussion(usize),
    Apply,
    /// One wave of the work graph: its units run concurrently, each in its own copy.
    Wave(usize),
    /// Merges a wave that ran more than one unit, before anything builds on it.
    Join(usize),
}

fn relative(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 512
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

impl Plan {
    pub fn validate(&self, config: &Config) -> Result<()> {
        ensure!(
            (3..=10).contains(&self.participants.len()),
            "collaboration needs 3..=10 participants"
        );
        let mut ids = BTreeSet::new();
        for p in &self.participants {
            ensure!(
                !p.id.is_empty()
                    && p.id.len() <= 64
                    && ![
                        "baseline",
                        "control",
                        "merged",
                        "parts",
                        "resolve",
                        "resolve-base"
                    ]
                    .contains(&p.id.to_ascii_lowercase().as_str())
                    && p.id
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                    && ids.insert(p.id.to_ascii_lowercase()),
                "invalid, reserved or duplicate participant ID"
            );
            let executes = |id: &str| config.agents.iter().any(|a| a.id == id && !a.routing_only);
            ensure!(
                p.agent.is_empty() || executes(&p.agent),
                "unknown or routing-only preferred agent: {}",
                p.agent
            );
            ensure!(
                p.fallback || !p.agent.is_empty(),
                "pinned participant needs an agent"
            );
            ensure!(
                p.allowed_agents.iter().all(|id| executes(id)),
                "unknown or routing-only allowed agent"
            );
            ensure!(
                p.agent.is_empty()
                    || p.allowed_agents.is_empty()
                    || p.allowed_agents.contains(&p.agent),
                "preferred agent must be allowed"
            );
            ensure!(
                p.paths.is_empty() || p.role == Role::Implementer,
                "only implementers can own paths"
            );
            ensure!(
                p.paths.len() <= 32 && p.paths.iter().all(|path| relative(path)),
                "owned paths must be at most 32 relative paths"
            );
        }
        let offset = usize::from(self.participants[0].role == Role::Coordinator);
        let workers = &self.participants[offset..];
        let implementers = workers
            .iter()
            .take_while(|p| p.role == Role::Implementer)
            .count();
        let rest = &workers[implementers..];
        ensure!(
            (1..=4).contains(&implementers)
                && (2..=5).contains(&rest.len())
                && rest.last().unwrap().role == Role::Integrator
                && rest[..rest.len() - 1]
                    .iter()
                    .all(|p| p.role == Role::Reviewer),
            "optional coordinator, then 1..=4 implementers, 1..=4 reviewers and one integrator required"
        );
        if let Some(discussion) = &self.discussion {
            ensure!(
                (1..=6).contains(&discussion.max_rounds),
                "discussion.max_rounds must be in 1..=6"
            );
        }
        if !self.parts.is_empty() {
            graph::waves(&self.parts, implementers)?;
            ensure!(
                self.parts
                    .iter()
                    .all(|part| !ids.contains(&part.id.to_ascii_lowercase())),
                "a part may not share a participant's ID"
            );
        }
        Ok(())
    }
    /// The waves the parts run in, as many units at once as there are implementer seats.
    /// `None` without parts; `validate` refuses parts that cannot be ordered.
    pub fn waves(&self) -> Option<Vec<Vec<Unit>>> {
        if self.parts.is_empty() {
            return None;
        }
        graph::waves(&self.parts, self.indices(Role::Implementer).len().max(1)).ok()
    }
    /// The team in one line, for the run that derived it rather than being handed it.
    pub fn summary(&self) -> String {
        let count = |role: Role| self.indices(role).len();
        let mut parts = vec![];
        if count(Role::Coordinator) > 0 {
            parts.push("coordinator".to_owned());
        }
        parts.push(format!("{} implementer(s)", count(Role::Implementer)));
        parts.push(format!("{} reviewer(s)", count(Role::Reviewer)));
        parts.push("integrator".to_owned());
        if let Some(discussion) = &self.discussion {
            parts.push(format!("{} discussion round(s)", discussion.max_rounds));
        }
        parts.join(", ")
    }
    fn indices(&self, role: Role) -> Vec<usize> {
        (0..self.participants.len())
            .filter(|i| self.participants[*i].role == role)
            .collect()
    }
    fn integrator(&self) -> usize {
        self.participants.len() - 1
    }
    fn steps(&self, apply: bool) -> Vec<Step> {
        let coordinator = self.participants[0].role == Role::Coordinator;
        let mut steps = vec![];
        let turn = |steps: &mut Vec<Step>, group: Vec<usize>| {
            steps.push(Step::Turn(group));
            if coordinator {
                steps.push(Step::Turn(vec![0]));
            }
        };
        if coordinator {
            steps.push(Step::Turn(vec![0]));
        }
        let implementers = self.indices(Role::Implementer);
        let waves = self.waves();
        // With parts, the waves have already been merged by the time anyone reviews.
        let merged = waves.is_none() && implementers.len() > 1;
        if let Some(waves) = &waves {
            for (index, wave) in waves.iter().enumerate() {
                steps.push(Step::Wave(index));
                if wave.len() > 1 {
                    steps.push(Step::Join(index));
                }
                if coordinator {
                    steps.push(Step::Turn(vec![0]));
                }
            }
        } else {
            turn(&mut steps, implementers);
        }
        if merged {
            steps.push(Step::Merge);
        }
        for reviewer in self.indices(Role::Reviewer) {
            turn(&mut steps, vec![reviewer]);
        }
        if let Some(discussion) = &self.discussion {
            steps.extend((1..=discussion.max_rounds).map(Step::Discussion));
            if merged {
                steps.push(Step::Merge);
            }
        }
        turn(&mut steps, vec![self.integrator()]);
        if apply {
            steps.push(Step::Apply);
        }
        steps
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptStatus {
    Running,
    Completed,
    Failed,
    Interrupted,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionResult {
    pub stage: usize,
    pub participant: String,
    pub worker_id: String,
    pub session_id: String,
    pub role: Role,
    #[serde(default)]
    pub turn: TurnKind,
    pub workspace: PathBuf,
    pub candidate: ExecutionCandidate,
    pub usage: Usage,
    pub outcome: Outcome,
    pub checks: Vec<CheckResult>,
    pub duration_ms: u64,
    pub response: String,
    pub status: AttemptStatus,
    pub error_kind: Option<ErrorKind>,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Blocked,
    Cancelled,
    Completed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: usize,
    pub stage: usize,
    pub from: String,
    pub to: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejected: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeRecord {
    pub stage: usize,
    pub sources: Vec<String>,
    pub conflicts: Vec<Conflict>,
    /// The wave of the work graph this merge joined.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wave: Option<usize>,
    /// Per part: files it changed outside the paths it declared. The evidence for (or
    /// against) trusting declared paths enough to run parts side by side.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub strays: BTreeMap<String, usize>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyStatus {
    Pending,
    Resolving,
    Resolved,
    Applied,
    Skipped,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Application {
    pub status: ApplyStatus,
    pub files: Vec<PathBuf>,
    pub conflicts: Vec<Conflict>,
    pub detail: Option<String>,
    /// Resolution workspaces prepared so far; each needs its own completed session.
    #[serde(default)]
    pub rounds: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub schema_version: u32,
    pub collaboration_id: String,
    pub root: PathBuf,
    pub task: String,
    pub plan: Plan,
    pub next_stage: usize,
    pub status: RunStatus,
    pub management: Option<Coordination>,
    pub sessions: Vec<SessionResult>,
    pub discovery_failures: Vec<String>,
    pub final_workspace: Option<PathBuf>,
    pub outcome: Outcome,
    pub error: Option<String>,
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default)]
    pub merges: Vec<MergeRecord>,
    #[serde(default)]
    pub apply_requested: bool,
    #[serde(default)]
    pub application: Option<Application>,
    /// Wall-clock time spent running, summed over every process that worked on it.
    #[serde(default)]
    pub elapsed_ms: u64,
}
impl Report {
    /// Completed, requested application reached the working tree or was not requested.
    pub fn finished(&self) -> bool {
        self.status == RunStatus::Completed
            && (!self.apply_requested
                || self
                    .application
                    .as_ref()
                    .is_some_and(|a| a.status == ApplyStatus::Applied))
    }
    fn completed(&self, stage: usize, participant: &str) -> bool {
        self.sessions.iter().any(|s| {
            s.stage == stage && s.participant == participant && s.status == AttemptStatus::Completed
        })
    }
    fn integration(&self) -> Option<&SessionResult> {
        self.sessions.iter().rev().find(|s| {
            s.role == Role::Integrator
                && s.turn == TurnKind::Scheduled
                && s.status == AttemptStatus::Completed
        })
    }
}

fn save(output: &Path, report: &Report) -> Result<()> {
    // Same-directory rename keeps either the old or the new complete checkpoint on disk.
    let mut temp = tempfile::NamedTempFile::new_in(output)?;
    serde_json::to_writer_pretty(&mut temp, report)?;
    temp.write_all(b"\n")?;
    temp.as_file().sync_all()?;
    temp.persist(output.join("report.json"))?;
    Ok(())
}
fn lock(output: &Path) -> Result<std::fs::File> {
    let path = output.join(".run.lock");
    ensure!(
        !path.is_symlink(),
        "collaboration lock must not be a symlink"
    );
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    file.try_lock()
        .context("another process is continuing this collaboration")?;
    Ok(file)
}

/// Where the units of one wave work, each in `<id>/`, and where a wider wave is merged.
fn wave_dir(output: &Path, wave: usize) -> PathBuf {
    output.join("parts").join(wave.to_string())
}
/// The tree a wave leaves behind: its one unit's workspace, or the merge of its units. Unit
/// IDs cannot contain `_`, so `_merged` is never a unit's.
fn wave_result(output: &Path, waves: &[Vec<Unit>], wave: usize) -> PathBuf {
    match waves[wave].as_slice() {
        [only] => wave_dir(output, wave).join(&only.id),
        _ => wave_dir(output, wave).join("_merged"),
    }
}
/// The tree a wave starts from.
fn wave_base(output: &Path, waves: &[Vec<Unit>], wave: usize) -> PathBuf {
    if wave == 0 {
        output.join("baseline")
    } else {
        wave_result(output, waves, wave - 1)
    }
}

/// Takes the first coordinator turn's division of the work, once, when Orochi can order it
/// and it lets something run side by side. Anything else keeps the team as it was.
fn adopt(cx: &Cx<'_>, report: &mut Report, proposal: serde_json::Value) {
    if !report.plan.parts.is_empty() {
        return;
    }
    let mut plan = report.plan.clone();
    plan.parts = match serde_json::from_value(proposal) {
        Ok(parts) => parts,
        Err(_) => {
            cx.say("The proposed parts could not be read; keeping the team as it was".into());
            return;
        }
    };
    let waves = match plan.validate(cx.config).and_then(|()| {
        plan.waves()
            .context("the proposed parts could not be ordered")
    }) {
        Ok(waves) => waves,
        Err(error) => {
            cx.say(format!("{error:#}; keeping the team as it was"));
            return;
        }
    };
    if waves.iter().all(|wave| wave.len() < 2) {
        cx.say(
            "Nothing in the proposed parts can run side by side; keeping the team as it was".into(),
        );
        return;
    }
    cx.say(format!("Order: {}", graph::summary(&waves)));
    report.plan = plan;
}

struct Cx<'a> {
    config: &'a Config,
    policies: &'a Registry,
    store: &'a Store,
    output: &'a Path,
    /// Where progress goes when someone other than a terminal's stderr is showing it.
    events: Option<crate::acp::EventSink>,
    /// Interrupts after this stop the run, even one that came between two waits.
    since: crate::interrupt::Since,
}
impl Cx<'_> {
    fn say(&self, text: String) {
        match &self.events {
            Some(events) => {
                let _ = events.send(crate::acp::ExecutionEvent::Progress(
                    crate::acp::Progress::Note(text),
                ));
            }
            None => eprintln!("{text}"),
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    config: &Config,
    policies: &Registry,
    store: &Store,
    root: &Path,
    task: &str,
    plan: &Plan,
    output: &Path,
    apply: bool,
    events: Option<crate::acp::EventSink>,
) -> Result<Report> {
    plan.validate(config)?;
    ensure!(
        !task.trim().is_empty() && task.len() <= 1_048_576,
        "invalid collaboration task"
    );
    ensure!(!output.exists(), "collaboration output already exists");
    let parent = output.parent().context("output needs a parent directory")?;
    std::fs::create_dir_all(parent)?;
    let parent = parent.canonicalize()?;
    let root = root.canonicalize()?;
    ensure!(
        !parent.starts_with(&root) || parent.starts_with(root.join(".orochi")),
        "output must be outside the source workspace or inside its .orochi directory"
    );
    let output = parent.join(output.file_name().context("invalid output path")?);
    std::fs::create_dir(&output)?;
    let _lock = lock(&output)?;
    copy_workspace(&root, &output.join("baseline"))?;
    let mut report = Report {
        schema_version: 2,
        collaboration_id: uuid::Uuid::new_v4().to_string(),
        root,
        task: task.into(),
        plan: plan.clone(),
        next_stage: 0,
        status: RunStatus::Running,
        management: None,
        sessions: vec![],
        discovery_failures: vec![],
        final_workspace: None,
        outcome: Outcome::Failure,
        error: None,
        messages: vec![],
        merges: vec![],
        apply_requested: apply,
        application: None,
        elapsed_ms: 0,
    };
    save(&output, &report)?;
    let cx = Cx {
        config,
        policies,
        store,
        output: &output,
        events,
        since: crate::interrupt::mark(),
    };
    if let Some(waves) = report.plan.waves() {
        cx.say(format!("Order: {}", graph::summary(&waves)));
    }
    drive(&cx, &mut report).await?;
    Ok(report)
}

pub async fn resume(
    config: &Config,
    policies: &Registry,
    store: &Store,
    root: &Path,
    output: &Path,
    apply: bool,
) -> Result<Report> {
    let output = output.canonicalize()?;
    let _lock = lock(&output)?;
    let checkpoint = output.join("report.json");
    ensure!(
        checkpoint.metadata()?.len() <= 64 * 1024 * 1024,
        "checkpoint exceeds 64 MiB"
    );
    let mut report: Report = serde_json::from_slice(&std::fs::read(checkpoint)?)?;
    ensure!(
        report.schema_version == 2,
        "unsupported collaboration checkpoint"
    );
    ensure!(
        report.root == root.canonicalize()?,
        "resume must use the original --cwd"
    );
    report.plan.validate(config)?;
    report.apply_requested |= apply;
    ensure!(
        report.next_stage <= report.plan.steps(report.apply_requested).len(),
        "invalid checkpoint stage"
    );
    ensure!(
        !report.task.trim().is_empty() && report.task.len() <= 1_048_576,
        "invalid saved task"
    );
    if report.finished()
        || report
            .application
            .as_ref()
            .is_some_and(|a| a.status == ApplyStatus::Skipped)
    {
        return Ok(report);
    }
    // A terminated process cannot mark its last in-flight request. Its partial text and
    // workspace remain useful evidence, never proof that the stage completed.
    for session in report
        .sessions
        .iter_mut()
        .filter(|s| s.status == AttemptStatus::Running)
    {
        session.status = AttemptStatus::Interrupted;
        session.outcome = Outcome::Cancelled;
        session.error_kind = Some(ErrorKind::Cancelled);
        session.error = Some("previous process stopped before checkpointing completion".into());
    }
    report.status = RunStatus::Running;
    report.error = None;
    let cx = Cx {
        config,
        policies,
        store,
        output: &output,
        events: None,
        since: crate::interrupt::mark(),
    };
    drive(&cx, &mut report).await?;
    Ok(report)
}

async fn drive(cx: &Cx<'_>, report: &mut Report) -> Result<()> {
    let started = std::time::Instant::now();
    // Save all ordinary failures, including local I/O and validation failures, as a
    // resumable stage. Cancellation stops here instead of spending quota on a fallback.
    let result = drive_steps(cx, report).await;
    report.elapsed_ms = report
        .elapsed_ms
        .saturating_add(started.elapsed().as_millis() as u64);
    if let Err(error) = result {
        let cancelled = error
            .downcast_ref::<AgentError>()
            .is_some_and(|e| e.kind == ErrorKind::Cancelled);
        report.status = if cancelled {
            RunStatus::Cancelled
        } else {
            RunStatus::Blocked
        };
        report.outcome = if cancelled {
            Outcome::Cancelled
        } else {
            Outcome::Failure
        };
        report.error = Some(format!("{error:#}"));
    }
    save(cx.output, report)
}

async fn drive_steps(cx: &Cx<'_>, report: &mut Report) -> Result<()> {
    loop {
        // Read again every stage: the first coordinator turn may divide the work.
        let steps = report.plan.steps(report.apply_requested);
        if report.next_stage >= steps.len() {
            break;
        }
        let stage = report.next_stage;
        // Merges and copies between stages never wait, so they would not hear it themselves.
        if cx.since.interrupted() {
            return Err(AgentError::new(ErrorKind::Cancelled, "interrupted between stages").into());
        }
        match &steps[stage] {
            Step::Turn(group) => {
                let turns = group
                    .iter()
                    .filter(|i| !report.completed(stage, &report.plan.participants[**i].id))
                    .map(|i| turn::scheduled(cx, report, stage, *i))
                    .collect::<Result<Vec<_>>>()?;
                turn::run_group(cx, report, stage, turns).await?;
            }
            Step::Merge => apply::merge_implementers(cx, report, stage)?,
            Step::Wave(wave) => {
                let turns = turn::wave(cx, report, stage, *wave)?;
                turn::run_group(cx, report, stage, turns).await?;
            }
            Step::Join(wave) => apply::join(cx, report, stage, *wave).await?,
            Step::Discussion(round) => {
                let turns = turn::discussion(cx, report, stage, *round)?;
                if turns.is_empty() {
                    // Without new messages, later rounds cannot have any either.
                    while matches!(steps.get(report.next_stage + 1), Some(Step::Discussion(_))) {
                        report.next_stage += 1;
                    }
                } else {
                    turn::run_group(cx, report, stage, turns).await?;
                }
            }
            Step::Apply => apply::apply(cx, report, stage).await?,
        }
        report.next_stage += 1;
        report.error = None;
        save(cx.output, report)?;
    }
    report.status = RunStatus::Completed;
    report.outcome = report
        .integration()
        .context("missing integrated result")?
        .outcome
        .clone();
    report.error = None;
    Ok(())
}
