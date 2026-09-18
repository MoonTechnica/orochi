//! Deriving a team from the task instead of being handed one.
//!
//! `Plan::validate` is the contract this has to satisfy: an optional coordinator, then 1..=4
//! implementers, 1..=4 reviewers and exactly one integrator, 3..=10 participants in that
//! order. Everything here decides how many of each, and what the implementers own.
use super::{Discussion, Participant, Plan, Role};
use crate::types::{Complexity, TaskDescriptor};
use std::{
    collections::BTreeSet,
    path::{Component, Path},
};

/// Top-level areas the task looks like it touches. Two implementers inside one directory
/// produce a merge conflict rather than twice the work, so this bounds how wide the team
/// can usefully be.
fn areas(task: &TaskDescriptor) -> Vec<String> {
    let mut seen = BTreeSet::new();
    for file in &task.candidate_files {
        let mut components = Path::new(file).components();
        let Some(Component::Normal(first)) = components.next() else {
            continue;
        };
        // A directory owns everything under it; a file at the root owns only itself.
        seen.insert(if components.next().is_some() {
            first.to_string_lossy().into_owned()
        } else {
            file.clone()
        });
    }
    seen.into_iter().filter(|a| a.len() <= 512).collect()
}

/// What the repository is made of, when the task named nothing. Work big enough to split is
/// exactly the work least likely to mention the files it will touch, so without this the
/// largest tasks would be the ones that never get a second implementer.
fn repo_areas(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return vec![];
    };
    let mut seen = BTreeSet::new();
    for entry in entries.flatten().take(1000) {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.')
            || name.len() > 512
            || matches!(
                name.as_str(),
                "node_modules" | "target" | "vendor" | "dist" | "build" | "venv"
            )
            || !entry.file_type().is_ok_and(|t| t.is_dir())
        {
            continue;
        }
        seen.insert(name);
    }
    seen.into_iter().take(32).collect()
}

fn size(coordinator: bool, implementers: usize, reviewers: usize) -> usize {
    usize::from(coordinator) + implementers + reviewers + 1
}

/// Grows towards a team the user asked for by seating whoever is safe to add: someone to hold
/// the plan, then implementers while there are still separate areas for them, then reviewers.
/// An implementer with nothing of its own to own only collides with the others.
fn fit(
    coordinator: &mut bool,
    implementers: &mut usize,
    reviewers: &mut usize,
    areas: usize,
    total: usize,
) {
    while size(*coordinator, *implementers, *reviewers) < total {
        if !*coordinator {
            *coordinator = true;
        } else if *implementers < areas.min(4) {
            *implementers += 1;
        } else if *reviewers < 4 {
            *reviewers += 1;
        } else {
            break;
        }
    }
    while size(*coordinator, *implementers, *reviewers) > total {
        if *reviewers > 1 {
            *reviewers -= 1;
        } else if *implementers > 1 {
            *implementers -= 1;
        } else if *coordinator {
            *coordinator = false;
        } else {
            break;
        }
    }
}

fn seat(id: &str, role: Role) -> Participant {
    Participant {
        id: id.into(),
        role,
        // Left to the scheduler: it is the part that knows quota, cooldown and cost, and it
        // already prices an agent another seat is running higher.
        agent: String::new(),
        model: None,
        reasoning: None,
        mode: None,
        fallback: true,
        allowed_agents: vec![],
        paths: vec![],
    }
}

/// A team for this task. Always valid: every count is clamped to what `Plan::validate` takes.
pub fn derive(task: &TaskDescriptor, root: &Path) -> Plan {
    let mut areas = areas(task);
    if areas.len() < 2 {
        areas = repo_areas(root);
    }
    let mut implementers = match task.complexity {
        Complexity::Extreme => 3,
        Complexity::Complex => 2,
        Complexity::Simple | Complexity::Normal => 1,
    }
    .min(areas.len().max(1))
    .clamp(1, 4);

    let structural = task.requires_architecture_change
        || matches!(task.task_type.as_str(), "architecture" | "migration");
    // One reviewer reads the code. A structural change also needs the shape read, separately
    // from whether the lines are right, and extreme work is where a missed case costs most.
    let mut reviewers =
        (1 + usize::from(structural) + usize::from(task.complexity == Complexity::Extreme))
            .clamp(1, 4);

    // One implementer answering to one reviewer has nothing to coordinate.
    let mut coordinator =
        implementers > 1 || task.long_horizon || task.complexity == Complexity::Extreme;

    if let Some(asked) = task.seats {
        fit(
            &mut coordinator,
            &mut implementers,
            &mut reviewers,
            areas.len().max(1),
            asked.clamp(3, 10),
        );
    }

    let mut participants = vec![];
    if coordinator {
        participants.push(seat("coordinator", Role::Coordinator));
    }
    for index in 0..implementers {
        let mut seat = seat(&format!("implement-{}", index + 1), Role::Implementer);
        // Splitting the tree only helps when there is a separate area for each of them;
        // otherwise they work over the whole tree and the merge sorts it out.
        if implementers > 1 && areas.len() >= implementers {
            seat.paths = areas
                .iter()
                .skip(index)
                .step_by(implementers)
                .take(32)
                .cloned()
                .collect();
        }
        participants.push(seat);
    }
    for index in 0..reviewers {
        participants.push(seat(&format!("review-{}", index + 1), Role::Reviewer));
    }
    participants.push(seat("integrate", Role::Integrator));

    Plan {
        participants,
        // Rounds of agents answering each other are worth their cost when the work is big
        // enough to have been split, or when the user asked them to work together.
        discussion: (task.collaborative
            || matches!(task.complexity, Complexity::Complex | Complexity::Extreme))
        .then(|| Discussion {
            max_rounds: if task.complexity == Complexity::Extreme {
                3
            } else {
                2
            },
        }),
    }
}
