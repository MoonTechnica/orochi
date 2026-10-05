//! Planning, carrying out and checking work, for both code and other deliverables.
use super::Steps;
use crate::types::{Complexity, TaskDescriptor};

pub(super) const DESIGN: usize = 0;
pub(super) const IMPLEMENT: usize = 1;
pub(super) const REVIEW: usize = 2;

pub(super) fn steps(mode: Steps, task: &TaskDescriptor) -> Vec<usize> {
    match mode {
        Steps::Solo => vec![],
        Steps::Team => vec![DESIGN, IMPLEMENT, REVIEW],
        Steps::Auto => match task.complexity {
            Complexity::Extreme => vec![DESIGN, IMPLEMENT, REVIEW],
            Complexity::Complex => vec![DESIGN, IMPLEMENT],
            _ if task.requires_architecture_change => vec![DESIGN, IMPLEMENT],
            _ => vec![],
        },
    }
}

pub(super) struct Phase {
    pub stage: usize,
    pub name: &'static str,
    pub instruction: &'static str,
    pub asks_less: bool,
}

impl Phase {
    pub fn new(stage: usize, task: &TaskDescriptor) -> Self {
        let coding = task.is_coding();
        let (name, instruction) = match stage {
            DESIGN if coding => (
                "design",
                "Plan the work first (設計 / architecture). Change nothing yet. Answer with what has to happen, the pieces involved, the risks, and how the result will be checked. Answer in the language the user used. If the work divides into parts with separate deliverables, end with one line holding only {\"parts\":[{\"id\":\"<lowercase letters, digits, ->\",\"brief\":\"what this part produces\",\"paths\":[\"<relative paths it writes>\"],\"after\":[\"<ids of parts whose results it needs>\"]}]}; parts that only need to agree on an interface can run together. Leave it out when the work does not divide.",
            ),
            DESIGN => (
                "plan",
                "Plan the work first. Change nothing yet. Identify the user's requested deliverable, the questions to answer, sources or materials needed, dependencies, uncertainties and acceptance criteria. Plan research, analysis, writing or discussion as the task requires; do not turn it into a software implementation task. Answer in the language the user used. If independent parts produce files that can be combined, end with one line holding only {\"parts\":[{\"id\":\"<lowercase letters, digits, ->\",\"brief\":\"what this part produces\",\"paths\":[\"<relative paths it writes>\"],\"after\":[\"<ids of parts whose findings or deliverables it needs>\"]}]}. Leave it out when the work does not divide into file deliverables.",
            ),
            IMPLEMENT => (
                match task.task_type.as_str() {
                    "investigation" => "research",
                    "documentation" => "write",
                    "review" => "analyze",
                    "analysis" => "analyze",
                    "planning" => "develop-plan",
                    "creative" => "create",
                    "discussion" => "discuss",
                    _ if coding => "implement",
                    _ => "execute",
                },
                if coding {
                    "Carry out the task, following the plan above. Verify the result yourself. Answer in the language the user used."
                } else {
                    "Carry out the user's task, following the plan above. Produce the requested findings, analysis, document or discussion outcome. Check evidence, reasoning, coverage and acceptance criteria; separate established facts from assumptions and unresolved questions. Edit files only when the user's requested deliverable calls for it. Answer in the language the user used."
                },
            ),
            REVIEW => (
                "review",
                "Review (レビュー) the work just done against the original request and the plan's acceptance criteria. Change nothing. Check correctness, evidence, reasoning, completeness and any task-specific checks. For research verify sources and conclusions; for documents check accuracy and coverage; for discussions check assumptions and unresolved disagreements. Report what is wrong or missing, most important first, in the language the user used. End with one line: \"VERDICT: fix\" when something must change, or \"VERDICT: ok\" when it is sound.",
            ),
            _ => unreachable!("unknown workflow stage"),
        };
        Self {
            stage,
            name,
            instruction,
            // A research or writing plan does not make its substantive work easier.
            asks_less: stage == IMPLEMENT && coding,
        }
    }

    pub fn profile(&self, task: &TaskDescriptor) -> TaskDescriptor {
        let mut profile = task.clone();
        profile.task_type = match self.stage {
            DESIGN if task.is_coding() => "architecture".into(),
            REVIEW => "review".into(),
            _ => task.task_type.clone(),
        };
        if self.stage != IMPLEMENT {
            // A model recommended for doing the work need not suit planning or checking it.
            profile.suited = None;
            profile.preferred = None;
        }
        profile
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn difficulty_controls_splitting_for_every_kind_of_deliverable() {
        let root = tempfile::tempdir().unwrap();
        for kind in crate::router::classifier::TASK_TYPES {
            let mut task = crate::router::profiler::profile("task", root.path());
            task.task_type = (*kind).into();
            task.requires_architecture_change = false;
            for (difficulty, expected) in [
                (Complexity::Simple, vec![]),
                (Complexity::Normal, vec![]),
                (Complexity::Complex, vec![DESIGN, IMPLEMENT]),
                (Complexity::Extreme, vec![DESIGN, IMPLEMENT, REVIEW]),
            ] {
                task.complexity = difficulty;
                assert_eq!(steps(Steps::Auto, &task), expected, "{kind}/{difficulty:?}");
                assert!(steps(Steps::Solo, &task).is_empty());
                assert_eq!(steps(Steps::Team, &task), [DESIGN, IMPLEMENT, REVIEW]);
            }
        }
    }

    #[test]
    fn non_coding_work_keeps_its_difficulty_capabilities_and_model_fit() {
        let root = tempfile::tempdir().unwrap();
        for kind in [
            "investigation",
            "documentation",
            "analysis",
            "planning",
            "creative",
            "discussion",
            "review",
            "general",
        ] {
            let mut task = crate::router::profiler::profile("task", root.path());
            task.task_type = kind.into();
            task.complexity = Complexity::Extreme;
            task.requires_web = true;
            task.suited = Some("fit-model".into());
            for stage in [DESIGN, IMPLEMENT, REVIEW] {
                let phase = Phase::new(stage, &task);
                let profile = phase.profile(&task);
                assert!(!phase.asks_less, "{kind}/{stage}");
                assert_eq!(profile.complexity, Complexity::Extreme);
                assert!(profile.requires_web);
                if stage == REVIEW {
                    assert_eq!(profile.task_type, "review");
                } else {
                    assert_eq!(profile.task_type, kind);
                }
                assert_eq!(profile.suited.is_some(), stage == IMPLEMENT);
            }
        }
    }
}
