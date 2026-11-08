//! Where the big things go: helpers' worktrees, a local decision model and the sandbox VM,
//! each settable (`orochi places`, the window's Storage page) and none of them moved.
use orochi::{
    config::{Config, SandboxClient, WorkspacesConfig},
    decision, places, workspaces,
};
use std::path::PathBuf;

#[test]
fn the_worktree_place_is_a_template_filled_in_per_repository() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("orochi");
    let other = dir.path().join("elsewhere/orochi");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    let repo = repo.canonicalize().unwrap();
    let with = |template: &str| WorkspacesConfig {
        root: Some(template.into()),
        ..Default::default()
    };
    // The default is the same template, inside the repository.
    assert_eq!(
        workspaces::sessions_dir(&WorkspacesConfig::default(), &repo),
        repo.join(".orochi/sessions")
    );
    // {name}: one folder per repository name, wherever the user says.
    assert_eq!(
        workspaces::sessions_dir(&with("/Volumes/Big/worktrees/{name}"), &repo),
        PathBuf::from("/Volumes/Big/worktrees/orochi")
    );
    // {repository}: beside the repository.
    assert_eq!(
        workspaces::sessions_dir(&with("{repository}-worktrees"), &repo),
        PathBuf::from(format!("{}-worktrees", repo.display()))
    );
    // A folder naming no variable gets {name}-{hash}, so two repositories never share one.
    let plain = with("/Volumes/Big/worktrees");
    let here = workspaces::sessions_dir(&plain, &repo);
    assert!(here.starts_with("/Volumes/Big/worktrees"));
    assert!(
        here.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("orochi-")
    );
    assert_ne!(here, workspaces::sessions_dir(&plain, &other));
    // A conversation's directory is under it, and what was opened before the move is found.
    assert!(workspaces::directory(&plain, &repo, "thread-1").starts_with(&here));
    assert_eq!(
        workspaces::sessions_dirs(&plain, &repo),
        vec![here, repo.join(".orochi/sessions")]
    );
}

#[test]
fn a_template_must_come_out_absolute_and_name_only_known_variables() {
    for good in [
        "/Volumes/Big/worktrees",
        "/Volumes/Big/{name}",
        "{repository}/../{name}-worktrees",
        "/x/{name}-{hash}",
    ] {
        workspaces::check_template(good).unwrap_or_else(|e| panic!("{good}: {e:#}"));
    }
    for bad in ["worktrees", "{name}/x", "/x/{branch}", "/x/{name", ""] {
        assert!(workspaces::check_template(bad).is_err(), "{bad}");
    }
}

#[test]
fn a_conversations_sandbox_is_told_from_a_projects_own_wherever_its_folder_is() {
    let session = orochi::sandbox::Project {
        root: PathBuf::from("/Volumes/Big/worktrees/orochi/abc123"),
        mounts: vec![PathBuf::from("/Users/me/orochi")],
        ..Default::default()
    };
    assert!(orochi::sandbox::ops::is_session(&session));
    let own = orochi::sandbox::Project {
        root: PathBuf::from("/Users/me/orochi"),
        ..Default::default()
    };
    assert!(!orochi::sandbox::ops::is_session(&own));
}

#[test]
fn a_place_must_be_an_absolute_directory_orochi_can_write_in() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let data = dir.path().join("data");
    let mut config = Config::default();
    assert!(
        places::set(
            &mut config,
            &path,
            &data,
            places::Which::Models,
            Some("relative/models".into()),
            false
        )
        .is_err()
    );
    let models = dir.path().join("models here");
    places::set(
        &mut config,
        &path,
        &data,
        places::Which::Models,
        Some(models.display().to_string()),
        false,
    )
    .unwrap();
    assert!(models.is_dir(), "the folder is made when it is chosen");
    let saved = Config::load(&path).unwrap();
    assert_eq!(saved.decision.models_dir.as_deref(), Some(models.as_path()));
    assert_eq!(decision::local::home(&data, &saved.decision), models);
    // Put back, a local model is looked for where it was before.
    places::set(
        &mut config,
        &path,
        &data,
        places::Which::Models,
        None,
        false,
    )
    .unwrap();
    assert_eq!(
        decision::local::home(&data, &Config::load(&path).unwrap().decision),
        data.join("decision")
    );
    // A relative path written by hand into the file is refused when it is read.
    std::fs::write(&path, "[workspaces]\nroot = \"worktrees\"\n").unwrap();
    assert!(Config::load(&path).is_err());
}

#[test]
fn every_lima_command_is_given_the_configured_lima_home() {
    let mut config = Config::default();
    config.sandbox.client = SandboxClient::Lima;
    config.sandbox.limactl = "/bin/echo".into();
    assert!(config.sandbox.lima_env().is_none());
    let home = PathBuf::from("/Volumes/Big/lima");
    config.sandbox.lima_home = Some(home.clone());
    assert_eq!(config.sandbox.lima_disk(), home);
    let command = orochi::sandbox::Incus::new(&config.sandbox)
        .command(&["list".into()])
        .unwrap();
    let env: Vec<_> = command.get_envs().collect();
    assert!(
        env.iter()
            .any(|(k, v)| *k == "LIMA_HOME" && v.map(PathBuf::from) == Some(home.clone())),
        "{env:?}"
    );
}

#[test]
fn the_places_list_says_where_each_one_is_and_which_is_the_default() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.sandbox.client = SandboxClient::Incus;
    let listed = places::read(&config, dir.path());
    let which: Vec<_> = listed.iter().map(|p| p.which).collect();
    assert_eq!(
        which,
        vec![
            places::Which::Worktrees,
            places::Which::Models,
            places::Which::Vm
        ]
    );
    assert!(listed.iter().all(|p| p.configured.is_none()));
    assert_eq!(
        listed[1].effective.as_deref(),
        Some(dir.path().join("decision").as_path())
    );
    assert!(
        !listed[2].applies,
        "the VM's home is Lima's, unused with a plain Incus client"
    );
}
