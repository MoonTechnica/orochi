//! Durable role handoffs. A logical role can be held by successive independent ACP sessions.
mod apply;
pub mod plan;
mod turn;
mod workspace;

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
    collections::BTreeSet,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    /// Participant indices that work concurrently in separate workspaces.
    Turn(Vec<usize>),
    Merge,
    Discussion(usize),
    Apply,
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
                    && !["baseline", "control", "merged", "resolve", "resolve-base"]
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
        Ok(())
    }
    /// Whether the work splits across separate workspaces. One implementer gains nothing from
    /// a workspace of its own, so this is exactly the line between what a console turn can do
    /// in the user's tree and what needs the collaboration machinery.
    pub fn splits_work(&self) -> bool {
        self.indices(Role::Implementer).len() > 1
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
        let merged = implementers.len() > 1;
        turn(&mut steps, implementers);
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

struct Cx<'a> {
    config: &'a Config,
    policies: &'a Registry,
    store: &'a Store,
    output: &'a Path,
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
    };
    save(&output, &report)?;
    let cx = Cx {
        config,
        policies,
        store,
        output: &output,
    };
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
    };
    drive(&cx, &mut report).await?;
    Ok(report)
}

async fn drive(cx: &Cx<'_>, report: &mut Report) -> Result<()> {
    // Save all ordinary failures, including local I/O and validation failures, as a
    // resumable stage. Cancellation stops here instead of spending quota on a fallback.
    if let Err(error) = drive_steps(cx, report).await {
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
    let steps = report.plan.steps(report.apply_requested);
    while report.next_stage < steps.len() {
        let stage = report.next_stage;
        match &steps[stage] {
            Step::Turn(group) => {
                let turns = group
                    .iter()
                    .filter(|i| !report.completed(stage, &report.plan.participants[**i].id))
                    .map(|i| turn::scheduled(cx, report, stage, *i))
                    .collect::<Result<Vec<_>>>()?;
                turn::run_group(cx, report, stage, turns).await?;
            }
            Step::Merge => apply::merge_implementers(cx.output, report, stage)?,
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
