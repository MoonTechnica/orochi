use orochi::workspaces::{Finish, Tools, Workspaces, directory};
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
    std::fs::write(dir.path().join("tracked"), "original").unwrap();
    std::fs::write(dir.path().join(".gitignore"), ".orochi/\nignored\n").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-m", "initial"]);
    dir
}
#[test]
fn workers_snapshot_dirty_files_without_changing_the_source_index_and_keep_results() {
    let repo = repository();
    let root = repo.path().canonicalize().unwrap();
    std::fs::write(root.join("tracked"), "staged").unwrap();
    git(&root, &["add", "tracked"]);
    std::fs::write(root.join("tracked"), "unstaged").unwrap();
    std::fs::write(root.join("new"), "new").unwrap();
    std::fs::write(root.join("ignored"), "ignore").unwrap();
    let index = std::fs::read(root.join(".git/index")).unwrap();
    let head = git(&root, &["rev-parse", "HEAD"]);
    let directory = directory(&root, "one");
    let mut state = Workspaces::open(&root, &directory).unwrap();
    let a = state.create("worker").unwrap();
    let b = state.create("worker").unwrap();
    assert_ne!(a, b);
    assert_eq!(
        std::fs::read_to_string(a.join("tracked")).unwrap(),
        "unstaged"
    );
    assert_eq!(std::fs::read_to_string(b.join("new")).unwrap(), "new");
    assert!(!a.join("ignored").exists());
    assert!(!a.join(".orochi").exists());
    std::fs::write(a.join("tracked"), "worker a").unwrap();
    assert_eq!(
        std::fs::read_to_string(b.join("tracked")).unwrap(),
        "unstaged"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("tracked")).unwrap(),
        "unstaged"
    );
    assert_eq!(git(&root, &["rev-parse", "HEAD"]), head);
    assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), index);
    let resumed = Workspaces::open(&root, &directory).unwrap();
    assert_eq!(resumed.workspaces.len(), 2);
    assert_eq!(
        std::fs::read_to_string(resumed.workspaces[0].root.join("tracked")).unwrap(),
        "worker a"
    );
    let other = Workspaces::open(&root, &orochi::workspaces::directory(&root, "two")).unwrap();
    assert_ne!(resumed.directory, other.directory);
    assert!(other.workspaces.is_empty());
}
#[test]
fn invalid_names_and_non_git_sources_fail_without_modifying_source() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = Workspaces::open(dir.path(), &directory(dir.path(), "one")).unwrap();
    for name in ["../escape", "/tmp/escape", "", "has space"] {
        assert!(state.create(name).is_err());
    }
    assert!(state.create("worker").is_err());
    assert!(state.workspaces.is_empty());
}
#[test]
fn a_session_started_inside_a_subdirectory_runs_workers_in_that_subdirectory() {
    let repo = repository();
    let root = repo.path().canonicalize().unwrap();
    std::fs::create_dir(root.join("package")).unwrap();
    std::fs::write(root.join("package/new"), "package").unwrap();
    let source = root.join("package");
    let mut state = Workspaces::open(&source, &directory(&source, "one")).unwrap();
    let worker = state.create("writer").unwrap();
    assert!(worker.ends_with("writer/package"));
    assert_eq!(
        std::fs::read_to_string(worker.join("new")).unwrap(),
        "package"
    );
}

#[test]
fn a_session_can_start_from_a_linked_worktree_with_external_git_metadata() {
    let repo = repository();
    let linked_parent = tempfile::tempdir().unwrap();
    let linked = linked_parent.path().join("linked");
    git(
        repo.path(),
        &["worktree", "add", "--detach", linked.to_str().unwrap()],
    );
    let linked = linked.canonicalize().unwrap();
    let mut state = Workspaces::open(&linked, &directory(&linked, "session")).unwrap();
    assert_eq!(
        state.git_common().unwrap(),
        repo.path().canonicalize().unwrap().join(".git")
    );
    let worker = state.create("implementer").unwrap();
    assert_eq!(
        git(
            &worker,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"]
        ),
        state.git_common().unwrap().display().to_string()
    );
    assert_eq!(
        std::fs::read_to_string(worker.join("tracked")).unwrap(),
        "original"
    );
}

fn worktrees(root: &Path) -> usize {
    git(root, &["worktree", "list", "--porcelain"])
        .lines()
        .filter(|l| l.starts_with("worktree "))
        .count()
}

#[test]
fn closing_a_worktree_merges_its_changes_three_way_into_the_tree_and_removes_it() {
    let repo = repository();
    let root = repo.path().canonicalize().unwrap();
    std::fs::write(root.join("lines"), "one\ntwo\nthree\n").unwrap();
    git(&root, &["add", "lines"]);
    git(&root, &["commit", "-m", "lines"]);
    let head = git(&root, &["rev-parse", "HEAD"]);
    let index = std::fs::read(root.join(".git/index")).unwrap();
    let directory = directory(&root, "one");
    let mut state = Workspaces::open(&root, &directory).unwrap();
    let worker = state.create("worker").unwrap();
    // The helper changes the last line and adds a file; meanwhile the user changes the first
    // line, and what the helper did lands on top of that without touching the user's edit.
    std::fs::write(worker.join("lines"), "one\ntwo\nTHREE\n").unwrap();
    std::fs::write(worker.join("added.txt"), "from the helper").unwrap();
    std::fs::write(root.join("lines"), "ONE\ntwo\nthree\n").unwrap();
    let closed = state
        .close(
            "worker",
            Finish::Merge,
            &Tools::default(),
            "Implement the thing",
        )
        .unwrap();
    assert_eq!(closed.how, "merge");
    assert_eq!(closed.changed, ["added.txt", "lines"]);
    assert_eq!(closed.merged, ["added.txt", "lines"]);
    assert!(closed.conflicts.is_empty() && closed.patch.is_none() && closed.error.is_none());
    assert_eq!(
        std::fs::read_to_string(root.join("lines")).unwrap(),
        "ONE\ntwo\nTHREE\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("added.txt")).unwrap(),
        "from the helper"
    );
    // The worktree is gone, the manifest forgot it, and the user's HEAD and index are as
    // they were: the change arrived as uncommitted edits, for the user to commit.
    assert!(!worker.exists());
    assert_eq!(worktrees(&root), 1);
    assert!(
        Workspaces::open(&root, &directory)
            .unwrap()
            .workspaces
            .is_empty()
    );
    assert_eq!(git(&root, &["rev-parse", "HEAD"]), head);
    assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), index);
    assert!(
        closed.describe().contains("2 files merged"),
        "{}",
        closed.describe()
    );

    // A worktree that changed nothing just goes.
    let idle = state.create("idle").unwrap();
    let closed = state
        .close("idle", Finish::Merge, &Tools::default(), "")
        .unwrap();
    assert!(closed.changed.is_empty() && closed.patch.is_none());
    assert!(!idle.exists());
    assert_eq!(worktrees(&root), 1);
}

#[test]
fn a_conflicting_or_discarded_worktree_is_kept_as_a_patch_and_removed() {
    let repo = repository();
    let root = repo.path().canonicalize().unwrap();
    std::fs::write(root.join("lines"), "one\ntwo\nthree\n").unwrap();
    git(&root, &["add", "lines"]);
    git(&root, &["commit", "-m", "lines"]);
    let directory = directory(&root, "one");
    let mut state = Workspaces::open(&root, &directory).unwrap();
    let worker = state.create("worker").unwrap();
    std::fs::write(worker.join("lines"), "one\ntwo\nTHREE\n").unwrap();
    std::fs::write(worker.join("added.txt"), "from the helper").unwrap();
    std::fs::write(root.join("lines"), "one\ntwo\nthree!\n").unwrap();
    let closed = state
        .close("worker", Finish::Merge, &Tools::default(), "")
        .unwrap();
    // The same line changed on both sides: nothing is written, not even the file that would
    // have merged cleanly, and the whole change waits as a patch.
    assert_eq!(closed.conflicts, ["lines"]);
    assert!(closed.merged.is_empty());
    assert!(!root.join("added.txt").exists());
    assert_eq!(
        std::fs::read_to_string(root.join("lines")).unwrap(),
        "one\ntwo\nthree!\n"
    );
    let patch = closed.patch.clone().unwrap();
    assert_eq!(patch, directory.join("worker.patch"));
    let text = std::fs::read_to_string(&patch).unwrap();
    assert!(
        text.contains("+THREE") && text.contains("added.txt"),
        "{text}"
    );
    assert!(!worker.exists());
    assert_eq!(worktrees(&root), 1);
    assert!(closed.describe().contains("conflicts in lines"));

    // Discarding keeps a patch too, under the next free name.
    let again = state.create("worker").unwrap();
    std::fs::write(again.join("other.txt"), "x").unwrap();
    let closed = state
        .close("worker", Finish::Discard, &Tools::default(), "")
        .unwrap();
    assert_eq!(
        closed.patch.as_deref(),
        Some(directory.join("worker-2.patch").as_path())
    );
    assert!(!again.exists());
    assert_eq!(worktrees(&root), 1);
}

#[test]
fn a_worktree_closed_as_a_pull_request_is_replayed_onto_head_pushed_and_opened_with_gh() {
    let repo = repository();
    let root = repo.path().canonicalize().unwrap();
    let remote = tempfile::tempdir().unwrap();
    git(remote.path(), &["init", "--bare", "-q"]);
    git(
        &root,
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    let branch = git(&root, &["branch", "--show-current"]);
    git(&root, &["push", "-q", "-u", "origin", &branch]);
    let log = root.join(".orochi/gh.log");
    std::fs::create_dir_all(root.join(".orochi")).unwrap();
    let gh = root.join(".orochi/gh");
    std::fs::write(
        &gh,
        format!(
            "#!/bin/sh\necho \"$@\" >> {}\necho https://example.test/pull/1\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&gh, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    // The user's uncommitted edit is in the helper's snapshot but must not reach the pull
    // request: only what the helper changed is replayed onto the current HEAD.
    std::fs::write(root.join("tracked"), "user's unfinished edit").unwrap();
    let directory = directory(&root, "one");
    let mut state = Workspaces::open(&root, &directory).unwrap();
    let worker = state.create("feature").unwrap();
    std::fs::write(worker.join("feature.txt"), "implemented").unwrap();
    let tools = Tools {
        gh: Some(gh.clone()),
        branch_prefix: "orochi/".into(),
    };
    let closed = state
        .close("feature", Finish::Pr, &tools, "Add the feature")
        .unwrap();
    assert_eq!(closed.error, None);
    assert_eq!(closed.branch.as_deref(), Some("orochi/feature"));
    assert_eq!(closed.pr.as_deref(), Some("https://example.test/pull/1"));
    assert!(closed.patch.is_none());
    let pushed = git(remote.path(), &["rev-parse", "orochi/feature"]);
    assert_eq!(
        git(remote.path(), &["show", "orochi/feature:feature.txt"]),
        "implemented"
    );
    assert_eq!(
        git(remote.path(), &["show", "orochi/feature:tracked"]),
        "original"
    );
    assert_eq!(
        git(remote.path(), &["rev-parse", "orochi/feature^"]),
        git(&root, &["rev-parse", "HEAD"])
    );
    assert_eq!(
        git(remote.path(), &["log", "-1", "--format=%s", &pushed]),
        "Add the feature"
    );
    let asked = std::fs::read_to_string(&log).unwrap();
    assert!(
        asked.contains("pr create --head orochi/feature --title Add the feature")
            && asked.contains(&format!("--base {branch}")),
        "{asked}"
    );
    assert!(!worker.exists());
    assert_eq!(worktrees(&root), 1);
    assert_eq!(
        std::fs::read_to_string(root.join("tracked")).unwrap(),
        "user's unfinished edit"
    );
    assert!(!root.join("feature.txt").exists());

    // Without `gh` the branch is still pushed, and the result says what is missing.
    let again = state.create("feature").unwrap();
    std::fs::write(again.join("second.txt"), "more").unwrap();
    let closed = state
        .close(
            "feature",
            Finish::Pr,
            &Tools {
                gh: None,
                branch_prefix: "orochi/".into(),
            },
            "",
        )
        .unwrap();
    assert_eq!(closed.branch.as_deref(), Some("orochi/feature-2"));
    assert!(closed.pr.is_none());
    assert!(
        closed.error.as_deref().unwrap().contains("gh"),
        "{closed:?}"
    );
    assert_eq!(
        git(remote.path(), &["show", "orochi/feature-2:second.txt"]),
        "more"
    );
    assert_eq!(worktrees(&root), 1);
}

#[test]
fn a_conversations_leftover_worktrees_are_closed_and_dead_sessions_are_pruned_but_live_ones_kept() {
    let repo = repository();
    let root = repo.path().canonicalize().unwrap();
    let one = directory(&root, "one");
    let mut first = Workspaces::open(&root, &one).unwrap();
    let a = first.create("a").unwrap();
    std::fs::write(a.join("a.txt"), "a").unwrap();
    first.create("b").unwrap();
    let closed = first.close_all(&Tools::default()).unwrap();
    assert_eq!(closed.len(), 2);
    assert_eq!(closed[0].how, "closed");
    assert_eq!(
        closed[0].patch.as_deref(),
        Some(one.join("a.patch").as_path())
    );
    assert!(closed[1].patch.is_none());
    assert_eq!(worktrees(&root), 1);
    assert!(!root.join("a.txt").exists(), "nothing is merged unasked");

    // Two more sessions: one held by a live process, one whose process is gone.
    let two = directory(&root, "two");
    let live = orochi::workspaces::hold(&two).unwrap().unwrap();
    let mut second = Workspaces::open(&root, &two).unwrap();
    let kept = second.create("kept").unwrap();
    let three = directory(&root, "three");
    let mut third = Workspaces::open(&root, &three).unwrap();
    let gone = third.create("gone").unwrap();
    std::fs::write(gone.join("g.txt"), "g").unwrap();
    assert_eq!(worktrees(&root), 3);
    let report = orochi::workspaces::prune(&root, &Tools::default()).unwrap();
    let pruned: Vec<_> = report.iter().map(|(d, _)| d.clone()).collect();
    assert!(pruned.contains(&three), "{pruned:?}");
    assert!(
        !pruned.contains(&one),
        "a session with nothing to close is not news: {pruned:?}"
    );
    assert!(
        !pruned.contains(&two),
        "a held session is left alone: {pruned:?}"
    );
    assert!(kept.exists());
    assert!(!gone.exists());
    assert!(three.join("gone.patch").is_file());
    assert!(
        one.join("a.patch").is_file(),
        "a patch keeps its session directory"
    );
    assert_eq!(worktrees(&root), 2);
    drop(live);
    orochi::workspaces::prune(&root, &Tools::default()).unwrap();
    assert!(!kept.exists());
    assert!(
        !two.exists(),
        "a dead session directory with nothing left in it goes too"
    );
    assert_eq!(worktrees(&root), 1);
}
