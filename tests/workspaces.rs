use orochi::workspaces::{Workspaces, directory};
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
