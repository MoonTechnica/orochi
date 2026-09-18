use crate::types::{Complexity, TaskDescriptor};

/// A seat at one turn: how it is named in the mailbox, what it is there for, and whether it
/// may change the workspace. The first seat always owns the work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Role {
    pub name: &'static str,
    /// What this seat is asked to do, in the words that go into its prompt.
    pub brief: &'static str,
    pub writes: bool,
}

const ARCHITECT: Role = Role {
    name: "architect",
    brief: "Work out the shape of this change before it is built: what has to happen, which pieces it touches, what breaks if it is done the obvious way, and how the result can be checked.",
    writes: false,
};
const RESEARCHER: Role = Role {
    name: "researcher",
    brief: "The request leaves things open. Read the repository and pin down what is actually there: where the relevant code lives, what already exists, and which reading of the request the code supports.",
    writes: false,
};
const REVIEWER: Role = Role {
    name: "reviewer",
    brief: "Read the work as it lands and say what is wrong with it: bugs, cases it misses, conventions it breaks, and anything elsewhere in the repository it quietly invalidates.",
    writes: false,
};

const PARTNER: Role = Role {
    name: "partner",
    brief: "Work through this together with the agent doing it: answer what it asks, say what you would do differently, and keep every message short and worth reading.",
    writes: false,
};

/// Seats for a discussion, in the order they join. Each one is a different way of being right
/// about the same question, so a panel is worth more than the same answer five times.
const PANEL: [Role; 5] = [
    Role {
        name: "skeptic",
        brief: "Argue with what the others propose: where it breaks, what it assumes, what it will cost when it is wrong.",
        writes: false,
    },
    Role {
        name: "architect",
        brief: "Hold the shape of the thing: how the pieces fit, what belongs where, and which decisions are hard to undo later.",
        writes: false,
    },
    Role {
        name: "simplifier",
        brief: "Cut it down: what can be dropped, what is being solved twice, and what the smallest version that still works looks like.",
        writes: false,
    },
    Role {
        name: "operator",
        brief: "Speak for whoever lives with this once it ships: what breaks at 3am, what cannot be observed, what is painful to change.",
        writes: false,
    },
    Role {
        name: "advocate",
        brief: "Speak for the person this is for: what they actually asked for, and whether what is proposed would feel like an answer to them.",
        writes: false,
    },
];

/// The most agents one turn ever seats, however many are asked for.
pub const MAX_SEATS: usize = PANEL.len() + 1;

/// Names the seats a task deserves, from the task itself rather than from a fixed pairing.
/// Most work is one agent; a second, read-only seat joins when the task is big, structural
/// or vague enough that being wrong costs more than the second opinion does.
pub fn seats(task: &TaskDescriptor) -> Vec<Role> {
    let lead = Role {
        name: match task.task_type.as_str() {
            "architecture" => "architect",
            "migration" => "migrator",
            "refactor" => "refactorer",
            "bug_fix" => "fixer",
            "review" => "reviewer",
            "documentation" => "writer",
            "test" => "tester",
            "investigation" => "investigator",
            "small_edit" => "editor",
            _ => "implementer",
        },
        brief: "Carry out the task and own every change.",
        writes: true,
    };
    // Asked for outright: seat as many agents as were asked for, each a different view.
    if task.collaborative {
        let wanted = task.seats.unwrap_or(2).clamp(2, MAX_SEATS);
        let lead = if task.task_type == "discussion" {
            // Nobody is building anything, so nobody needs to write.
            Role {
                name: "facilitator",
                brief: "Run the discussion and report what came out of it.",
                writes: false,
            }
        } else {
            lead
        };
        let mut seats = vec![lead];
        seats.extend(PANEL.iter().take(wanted - 1).cloned());
        if seats.len() == 2 {
            seats[1] = PARTNER;
        }
        return seats;
    }
    let structural = task.requires_architecture_change
        || matches!(task.task_type.as_str(), "architecture" | "migration");
    let worth_it = !matches!(task.task_type.as_str(), "small_edit" | "documentation")
        && task.complexity != Complexity::Simple
        && (structural
            || task.long_horizon
            || matches!(task.complexity, Complexity::Complex | Complexity::Extreme));
    if !worth_it {
        return vec![lead];
    }
    let mate = if structural && lead.name != "architect" {
        ARCHITECT
    } else if task.ambiguity >= 0.6 {
        RESEARCHER
    } else {
        REVIEWER
    };
    vec![lead, mate]
}
