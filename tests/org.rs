use orochi::{
    config::{Config, LinkConfig, OrgConfig, RoleConfig},
    org::{Member, Org, Project, linked, roles},
    workspaces::{Finish, Tools, Workspaces, directory},
};
use std::{path::Path, process::Command};

fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}
fn repository() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init"]);
    git(dir.path(), &["config", "user.name", "Test"]);
    git(dir.path(), &["config", "user.email", "test@localhost"]);
    std::fs::write(dir.path().join("tracked"), "original\n").unwrap();
    std::fs::write(dir.path().join(".gitignore"), ".orochi/\n").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-m", "initial"]);
    dir
}
fn member(name: &str, role: &str, parent: Option<&str>, lead: bool) -> Member {
    Member {
        name: name.into(),
        role: Some(role.into()),
        parent: parent.map(str::to_owned),
        lead,
        writes: true,
    }
}
fn company() -> OrgConfig {
    OrgConfig {
        enabled: true,
        max_depth: 3,
        max_members: 12,
        max_reports: 3,
        roles: vec![
            RoleConfig {
                name: "cto".into(),
                lead: true,
                writes: true,
                may_hire: vec!["engineering-manager".into(), "reviewer".into()],
                ..Default::default()
            },
            RoleConfig {
                name: "engineering-manager".into(),
                lead: true,
                writes: true,
                may_hire: vec!["engineer".into(), "reviewer".into()],
                max_reports: Some(2),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

#[test]
fn the_default_organization_is_flat_and_only_the_lead_in_the_users_tree_may_hire() {
    let config = OrgConfig::default();
    assert!(!config.enabled && config.cost_gate && config.max_depth == 1);
    let org = Org::default();
    assert!(org.hire(&config, None, Some("engineer")).is_ok());
    let mut org = Org::default();
    org.add(member("api", "lead", None, true));
    let refused = org
        .hire(&config, Some("api"), Some("engineer"))
        .unwrap_err();
    assert!(refused.contains("org.enabled"), "{refused}");
}

#[test]
fn a_role_may_hire_only_the_roles_its_definition_names() {
    let config = company();
    let mut org = Org::default();
    org.add(member("cto", "cto", None, true));
    assert!(
        org.hire(&config, Some("cto"), Some("engineering-manager"))
            .is_ok()
    );
    let refused = org
        .hire(&config, Some("cto"), Some("engineer"))
        .unwrap_err();
    assert!(
        refused.contains("may hire engineering-manager, reviewer"),
        "{refused}"
    );
    let unknown = org
        .hire(&config, Some("cto"), Some("accountant"))
        .unwrap_err();
    assert!(
        unknown.contains("there is no role `accountant`"),
        "{unknown}"
    );
}

#[test]
fn depth_headcount_and_fan_out_each_stop_a_hire_and_name_their_setting() {
    let mut config = company();
    let mut org = Org::default();
    org.add(member("cto", "cto", None, true));
    org.add(member("web", "lead", Some("cto"), true));
    // Level 3 is the deepest: an engineer fits there, a lead there could hire nobody.
    assert!(org.hire(&config, Some("web"), Some("engineer")).is_ok());
    let deep = org.hire(&config, Some("web"), Some("lead")).unwrap_err();
    assert!(deep.contains("org.max_depth"), "{deep}");

    config.max_reports = 2;
    org.add(member("e1", "engineer", Some("web"), false));
    org.add(member("e2", "engineer", Some("web"), false));
    let full = org
        .hire(&config, Some("web"), Some("engineer"))
        .unwrap_err();
    assert!(full.contains("org.max_reports"), "{full}");

    config.max_members = 4;
    let crowded = org
        .hire(&config, Some("cto"), Some("reviewer"))
        .unwrap_err();
    assert!(crowded.contains("org.max_members"), "{crowded}");
}

#[test]
fn an_agent_that_is_not_a_lead_cannot_hire() {
    let config = company();
    let mut org = Org::default();
    org.add(member("e1", "engineer", None, false));
    let refused = org.hire(&config, Some("e1"), Some("reviewer")).unwrap_err();
    assert!(refused.contains("not a lead"), "{refused}");
}

#[test]
fn the_chart_depth_and_subtree_follow_who_reports_to_whom() {
    let mut org = Org::default();
    org.add(member("cto", "cto", None, true));
    org.add(member("web", "engineering-manager", Some("cto"), true));
    org.add(member("api", "engineering-manager", Some("cto"), true));
    org.add(member("e1", "engineer", Some("web"), false));
    assert_eq!(org.depth("cto"), 1);
    assert_eq!(org.depth("e1"), 3);
    assert_eq!(org.depth("nobody"), 0);
    let below: Vec<_> = org
        .below(Some("web"))
        .iter()
        .map(|m| m.name.clone())
        .collect();
    assert_eq!(below, ["e1"]);
    assert_eq!(org.below(None).len(), 4);
    assert!(org.within("e1", Some("cto")));
    assert!(!org.within("e1", Some("api")));
    assert!(org.within("api", None));
    let chart = org.chart();
    assert_eq!(chart[0], (0, "cto (cto)".into()));
    assert!(chart.contains(&(2, "e1 (engineer)".into())));
}

#[test]
fn a_configured_role_replaces_a_built_in_one_of_the_same_name() {
    let config = OrgConfig {
        roles: vec![RoleConfig {
            name: "reviewer".into(),
            instruction: "Be strict.".into(),
            writes: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let all = roles(&config);
    let reviewer = all.iter().find(|r| r.name == "reviewer").unwrap();
    assert_eq!(reviewer.instruction, "Be strict.");
    assert!(reviewer.writes);
    assert_eq!(all.iter().filter(|r| r.name == "reviewer").count(), 1);
    assert!(all.iter().any(|r| r.name == "engineer" && r.writes));
}

#[test]
fn configuration_rejects_an_organization_that_names_a_role_that_does_not_exist() {
    let mut config = Config::default();
    config.org.roles = vec![RoleConfig {
        name: "cto".into(),
        lead: true,
        may_hire: vec!["accountant".into()],
        ..Default::default()
    }];
    let error = config.validate().unwrap_err().to_string();
    assert!(error.contains("accountant"), "{error}");
    config.org.roles[0].may_hire = vec!["engineer".into()];
    config.validate().unwrap();
    config.manager.drive = "committee".into();
    assert!(
        config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("manager.drive")
    );
}

#[test]
fn a_reports_worktree_is_cut_from_its_leads_worktree_and_merges_back_into_it() {
    let repo = repository();
    let root = repo.path().canonicalize().unwrap();
    let directory = directory(&Default::default(), &root, "org");
    let mut state = Workspaces::open(&root, &directory).unwrap();
    let lead = state.create_under("api", "the api team", None).unwrap();
    // The lead's own work in progress is what its report starts from.
    std::fs::write(lead.join("tracked"), "original\nlead edit\n").unwrap();
    let report = state
        .create_under("e1", "an engineer", Some("api"))
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(report.join("tracked")).unwrap(),
        "original\nlead edit\n"
    );
    assert_eq!(state.depth("api"), 0);
    assert_eq!(state.depth("e1"), 1);
    assert_eq!(state.descendants("api"), ["e1"]);

    std::fs::write(report.join("report.txt"), "from the engineer\n").unwrap();
    let closed = state
        .close("e1", Finish::Merge, &Tools::default(), "")
        .unwrap();
    assert!(closed.conflicts.is_empty(), "{closed:?}");
    // It landed in the lead's worktree and nowhere else.
    assert_eq!(
        std::fs::read_to_string(lead.join("report.txt")).unwrap(),
        "from the engineer\n"
    );
    assert!(!root.join("report.txt").exists());
    assert_eq!(
        std::fs::read_to_string(root.join("tracked")).unwrap(),
        "original\n"
    );
}

#[test]
fn a_leads_report_cannot_open_a_pull_request_and_the_sweep_closes_reports_before_their_lead() {
    let repo = repository();
    let root = repo.path().canonicalize().unwrap();
    let directory = directory(&Default::default(), &root, "sweep");
    let mut state = Workspaces::open(&root, &directory).unwrap();
    state.create_under("api", "", None).unwrap();
    let report = state.create_under("e1", "", Some("api")).unwrap();
    std::fs::write(report.join("work.txt"), "unfinished\n").unwrap();

    let refused = state
        .close("e1", Finish::Pr, &Tools::default(), "")
        .unwrap();
    assert!(refused.error.unwrap().contains("pull request"));
    assert!(refused.patch.is_some(), "what did not land is kept");

    let again = state.create_under("e2", "", Some("api")).unwrap();
    std::fs::write(again.join("more.txt"), "x\n").unwrap();
    let closed = state.close_all(&Tools::default()).unwrap();
    let order: Vec<_> = closed.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(order, ["e2", "api"]);
    assert!(state.workspaces.is_empty());
}

fn project(name: &str, root: &str) -> Project {
    Project {
        name: name.into(),
        root: root.into(),
    }
}

#[test]
fn a_link_lets_one_project_message_another_in_the_direction_it_names_and_no_other() {
    let web = project("web", "/work/web");
    let billing = project("billing", "/work/billing");
    let mut config = OrgConfig::default();
    assert!(
        !linked(&config, &web, &billing),
        "without a link nothing crosses"
    );
    config.links = vec![LinkConfig {
        from: "web".into(),
        to: "billing".into(),
    }];
    assert!(linked(&config, &web, &billing));
    assert!(!linked(&config, &billing, &web), "a link has a direction");
    assert!(!linked(&config, &web, &project("secret", "/work/secret")));
    // Names are matched without regard to case, and a folder names a project too.
    config.links = vec![LinkConfig {
        from: "WEB".into(),
        to: "/work/billing/".into(),
    }];
    assert!(linked(&config, &web, &billing));
}

#[test]
fn a_wildcard_link_reaches_every_other_project_but_never_the_project_itself() {
    let web = project("web", "/work/web");
    let config = OrgConfig {
        links: vec![LinkConfig {
            from: "*".into(),
            to: "*".into(),
        }],
        ..Default::default()
    };
    assert!(linked(&config, &web, &project("billing", "/work/billing")));
    assert!(
        !linked(&config, &web, &web),
        "a project does not message itself"
    );
}
