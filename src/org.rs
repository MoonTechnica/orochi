//! The shape of a conversation's agent organization (`docs/org-design.md`): which roles
//! exist, who reports to whom, and whether a lead may hire one more. Orochi decides every hire
//! from these rows; an agent only asks, and each refusal is a sentence it can repeat.
use crate::config::OrgConfig;
use serde::{Deserialize, Serialize};

/// A role Orochi ships. A configured role of the same name replaces it.
pub struct BuiltinRole {
    pub name: &'static str,
    pub instruction: &'static str,
    pub writes: bool,
    pub lead: bool,
}

pub const BUILTIN: &[BuiltinRole] = &[
    BuiltinRole {
        name: "researcher",
        instruction: "You investigate and report what you find, with where you found it. You change nothing.",
        writes: false,
        lead: false,
    },
    BuiltinRole {
        name: "designer",
        instruction: "You plan the work: the pieces, the risks and how it will be checked. You change nothing.",
        writes: false,
        lead: false,
    },
    BuiltinRole {
        name: "engineer",
        instruction: "You implement your part in your own worktree and verify it yourself.",
        writes: true,
        lead: false,
    },
    BuiltinRole {
        name: "reviewer",
        instruction: "You review the work against what was asked and report what is wrong or missing, most important first. You change nothing.",
        writes: false,
        lead: false,
    },
    BuiltinRole {
        name: "lead",
        instruction: "You lead a team on one part of the work: divide it, hire the roles it needs, integrate what your reports produce in your own worktree, and report up only when it is sound.",
        writes: true,
        lead: true,
    },
];

/// A project as a link names it: by the name the window shows, or by its folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub name: String,
    pub root: String,
}

impl Project {
    /// Whether a link's `from` or `to` names this project (`*` names every one).
    pub fn is(&self, pattern: &str) -> bool {
        let pattern = pattern.trim();
        pattern == "*"
            || pattern.eq_ignore_ascii_case(&self.name)
            || pattern.trim_end_matches('/') == self.root.trim_end_matches('/')
    }
}

/// Whether `from` may message `to`: only a configured link says so, in the direction it
/// names (`docs/org-design.md` D12).
pub fn linked(config: &OrgConfig, from: &Project, to: &Project) -> bool {
    from.root != to.root
        && config
            .links
            .iter()
            .any(|link| from.is(&link.from) && to.is(&link.to))
}

/// A role as the organization uses it: the built-in ones, with configured ones added or
/// replacing them by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Role {
    pub name: String,
    pub instruction: String,
    pub writes: bool,
    pub lead: bool,
    pub may_hire: Vec<String>,
    pub max_reports: Option<usize>,
}

pub fn roles(config: &OrgConfig) -> Vec<Role> {
    let mut roles: Vec<Role> = BUILTIN
        .iter()
        .filter(|b| !config.roles.iter().any(|r| r.name == b.name))
        .map(|b| Role {
            name: b.name.into(),
            instruction: b.instruction.into(),
            writes: b.writes,
            lead: b.lead,
            may_hire: vec![],
            max_reports: None,
        })
        .collect();
    roles.extend(config.roles.iter().map(|r| Role {
        name: r.name.clone(),
        instruction: r.instruction.clone(),
        writes: r.writes,
        lead: r.lead,
        may_hire: r.may_hire.clone(),
        max_reports: r.max_reports,
    }));
    roles
}

/// What the lead in the user's tree is told when the whole request is its to divide
/// (`manager.drive = "lead"`): that the division is its decision, what it may hire, and the
/// limits Orochi will hold it to. The roles are the organization's, not a fixed pipeline.
pub fn lead_brief(config: &OrgConfig) -> String {
    let roles = roles(config);
    let hireable: Vec<String> = roles
        .iter()
        .filter(|r| config.enabled || !r.lead)
        .map(|r| {
            format!(
                "`{}` ({}{}; {})",
                r.name,
                if r.writes { "writes" } else { "reads only" },
                if r.lead { ", leads a team" } else { "" },
                r.instruction.split('.').next().unwrap_or("").trim()
            )
        })
        .collect();
    let mut text = String::from(
        "You lead this work and the user has given it to you whole: how it divides is your decision, not a fixed sequence. If one agent can do it, do it yourself — every helper is a whole session. Otherwise keep a plan with set_plan and hire with start_agent, naming a role",
    );
    text.push_str(&format!(". Roles: {}.", hireable.join("; ")));
    if config.enabled && config.max_depth > 1 {
        text.push_str(&format!(
            " A part that is itself a team's work goes to a lead, which hires its own reports (up to level {}); you review what a lead hands up, and only you write the user's tree.",
            config.max_depth
        ));
    }
    text.push_str(&format!(
        " Limits Orochi holds you to: {} members in all, {} direct reports each. A writing report works in a worktree cut from yours; judge it with merge_trial and apply_merge, never with git. Tell the user what you started, keep working as reports arrive, and answer the user yourself once the work is sound.",
        config.max_members, config.max_reports
    ));
    text
}

/// One agent of the organization below the lead in the user's tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub name: String,
    pub role: Option<String>,
    /// The lead that hired it; none for a report of the lead in the user's tree.
    pub parent: Option<String>,
    /// It may start reports of its own.
    pub lead: bool,
    pub writes: bool,
}

/// Who is working for whom, in one conversation.
#[derive(Debug, Default, Clone)]
pub struct Org {
    members: Vec<Member>,
}

impl Org {
    pub fn members(&self) -> &[Member] {
        &self.members
    }
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }
    pub fn len(&self) -> usize {
        self.members.len()
    }
    pub fn get(&self, name: &str) -> Option<&Member> {
        self.members.iter().find(|m| m.name == name)
    }
    pub fn add(&mut self, member: Member) {
        self.members.retain(|m| m.name != member.name);
        self.members.push(member);
    }
    pub fn remove(&mut self, name: &str) -> Option<Member> {
        let index = self.members.iter().position(|m| m.name == name)?;
        Some(self.members.remove(index))
    }
    pub fn clear(&mut self) {
        self.members.clear();
    }
    /// Levels below the lead in the user's tree: 1 for its own reports. A name that is no
    /// member is the lead itself, at 0.
    pub fn depth(&self, name: &str) -> usize {
        let mut depth = 0;
        let mut at = self.get(name);
        while let Some(member) = at {
            depth += 1;
            if depth > self.members.len() {
                break;
            }
            at = member.parent.as_deref().and_then(|p| self.get(p));
        }
        depth
    }
    /// The direct reports of `parent` (none: the lead in the user's tree).
    pub fn reports(&self, parent: Option<&str>) -> Vec<&Member> {
        self.members
            .iter()
            .filter(|m| m.parent.as_deref() == parent)
            .collect()
    }
    /// Every member below `name`, at any depth, deepest last.
    pub fn below(&self, name: Option<&str>) -> Vec<&Member> {
        let mut found: Vec<&Member> = vec![];
        let mut queue: Vec<Option<&str>> = vec![name];
        while let Some(parent) = queue.pop() {
            for member in self.reports(parent) {
                if !found.iter().any(|f| f.name == member.name) {
                    found.push(member);
                    queue.push(Some(member.name.as_str()));
                }
            }
        }
        found
    }
    /// Whether `name` is `ancestor` or works somewhere under it (none is the lead in the
    /// user's tree, above everyone).
    pub fn within(&self, name: &str, ancestor: Option<&str>) -> bool {
        let Some(ancestor) = ancestor else {
            return true;
        };
        let mut at = Some(name);
        let mut steps = 0;
        while let Some(current) = at {
            if current == ancestor {
                return true;
            }
            steps += 1;
            if steps > self.members.len() + 1 {
                break;
            }
            at = self.get(current).and_then(|m| m.parent.as_deref());
        }
        false
    }

    /// Whether `requester` (none: the lead in the user's tree) may hire one more agent in
    /// `role` now. The refusal names the setting that decided it.
    pub fn hire(
        &self,
        config: &OrgConfig,
        requester: Option<&str>,
        role: Option<&str>,
    ) -> Result<Role, String> {
        let known = roles(config);
        let wanted = match role {
            None => None,
            Some(name) => Some(known.iter().find(|r| r.name == name).cloned().ok_or_else(
                || {
                    format!(
                        "there is no role `{name}`; the roles are {}",
                        known
                            .iter()
                            .map(|r| r.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                },
            )?),
        };
        let hire = wanted.clone().unwrap_or_else(|| Role {
            name: String::new(),
            instruction: String::new(),
            writes: false,
            lead: false,
            may_hire: vec![],
            max_reports: None,
        });
        if self.members.len() >= config.max_members {
            return Err(format!(
                "already {} agents on this conversation (org.max_members)",
                self.members.len()
            ));
        }
        let depth = match requester {
            None => 1,
            Some(name) => {
                if !config.enabled {
                    return Err(
                        "only the lead in the user's tree starts helpers here (org.enabled is off)"
                            .into(),
                    );
                }
                let Some(member) = self.get(name) else {
                    return Err(format!("`{name}` is not part of this organization"));
                };
                if !member.lead {
                    return Err(format!(
                        "`{name}` is not a lead and starts nobody; do the work yourself"
                    ));
                }
                if let Some(mine) = member
                    .role
                    .as_deref()
                    .and_then(|r| known.iter().find(|k| k.name == r))
                    && !mine.may_hire.is_empty()
                    && !wanted
                        .as_ref()
                        .is_some_and(|w| mine.may_hire.contains(&w.name))
                {
                    return Err(format!(
                        "a {} may hire {}, not {}",
                        mine.name,
                        mine.may_hire.join(", "),
                        role.unwrap_or("an unnamed role")
                    ));
                }
                self.depth(name) + 1
            }
        };
        if depth > config.max_depth {
            return Err(format!(
                "that would be level {depth} and the organization stops at {} (org.max_depth)",
                config.max_depth
            ));
        }
        if hire.lead && depth >= config.max_depth {
            return Err(format!(
                "a lead at level {depth} could hire nobody: the organization stops at {} (org.max_depth)",
                config.max_depth
            ));
        }
        let limit = requester
            .and_then(|name| self.get(name))
            .and_then(|m| m.role.as_deref())
            .and_then(|r| known.iter().find(|k| k.name == r))
            .and_then(|r| r.max_reports)
            .unwrap_or(config.max_reports)
            .min(config.max_reports);
        let reports = self.reports(requester).len();
        if reports >= limit {
            return Err(format!(
                "already {reports} reports under {} (org.max_reports)",
                requester.unwrap_or("the lead")
            ));
        }
        Ok(wanted.unwrap_or(hire))
    }

    /// What a member is told about where it sits: whom it reports to, how deep that is, and, for
    /// a lead, whom it may hire and how much room is left.
    pub fn brief(&self, config: &OrgConfig, name: &str, role: &Role, manager: &str) -> String {
        let depth = self.depth(name);
        let mut text = format!(
            "Organization: you are `{name}`, a {} at level {depth} of {}, reporting to `{manager}`. Your result goes to `{manager}`, never past it.",
            role.name, config.max_depth
        );
        if role.lead {
            let known = roles(config);
            let hireable: Vec<String> = known
                .iter()
                .filter(|r| role.may_hire.is_empty() || role.may_hire.contains(&r.name))
                .map(|r| {
                    format!(
                        "`{}` ({})",
                        r.name,
                        r.instruction.split('.').next().unwrap_or("")
                    )
                })
                .collect();
            let limit = role
                .max_reports
                .unwrap_or(config.max_reports)
                .min(config.max_reports);
            text.push_str(&format!(
                " You may hire, with start_agent and a role: {}. You have {} of {limit} reports and the conversation {} of {} members.",
                hireable.join("; "),
                self.reports(Some(name)).len(),
                self.len(),
                config.max_members
            ));
        }
        text
    }

    /// The organization as indented lines, for a listing: name, role and depth.
    pub fn chart(&self) -> Vec<(usize, String)> {
        let mut lines = vec![];
        fn walk(org: &Org, parent: Option<&str>, depth: usize, lines: &mut Vec<(usize, String)>) {
            for member in org.reports(parent) {
                let role = member
                    .role
                    .as_deref()
                    .map(|r| format!(" ({r})"))
                    .unwrap_or_default();
                lines.push((depth, format!("{}{role}", member.name)));
                walk(org, Some(&member.name), depth + 1, lines);
            }
        }
        walk(self, None, 0, &mut lines);
        lines
    }
}
