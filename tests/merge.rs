//! Merge trials and applies of a conversation's worktrees (`docs/manager-design.md` §4.5).
use orochi::{
    merge::{self, State},
    workspaces::{Workspaces, directory},
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
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn repository() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init"]);
    git(dir.path(), &["config", "user.name", "Test"]);
    git(dir.path(), &["config", "user.email", "test@localhost"]);
    std::fs::write(dir.path().join("tracked"), "a\nb\nc\nd\ne\n").unwrap();
    std::fs::write(dir.path().join(".gitignore"), ".orochi/\n").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-m", "initial"]);
    dir
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn a_trial_merges_a_worktree_against_the_tree_as_it_is_now_and_apply_writes_only_the_difference() {
    let repo = repository();
    let root = repo.path().canonicalize().unwrap();
    let session = directory(&root, "one");
    let mut work = Workspaces::open(&root, &session).unwrap();
    let worktree = work.create("backend").unwrap();
    // The helper edits the end of the file and adds one; the user edits its start meanwhile,
    // uncommitted, which the trial has to see without being told.
    std::fs::write(worktree.join("tracked"), "a\nb\nc\nd\nE\n").unwrap();
    std::fs::write(worktree.join("feature"), "new\n").unwrap();
    std::fs::write(root.join("tracked"), "A\nb\nc\nd\ne\n").unwrap();
    let head = git(&root, &["rev-parse", "HEAD"]);
    let index = std::fs::read(root.join(".git/index")).unwrap();

    let backend = work.find("backend").unwrap();
    let mut trial = merge::trial(&root, backend, &session, 1).unwrap();
    assert_eq!(trial.id, "backend-1");
    assert_eq!(trial.state, State::Clean, "{trial:?}");
    assert_eq!(trial.files, ["feature", "tracked"]);
    assert_eq!(
        trial.overlap,
        ["tracked"],
        "both sides changed it, which a clean merge cannot vouch for"
    );
    assert!(trial.conflicts.is_empty());
    assert!(trial.root.starts_with(&session), "{}", trial.root.display());
    assert_eq!(read(&trial.root.join("tracked")), "A\nb\nc\nd\nE\n");
    assert_eq!(
        read(&root.join("tracked")),
        "A\nb\nc\nd\ne\n",
        "a trial writes nothing to the user's tree"
    );
    assert!(!root.join("feature").exists());

    let written = merge::apply(&root, &session, &mut trial).unwrap();
    assert_eq!(trial.state, State::Applied);
    assert_eq!(written.len(), 2);
    assert_eq!(read(&root.join("tracked")), "A\nb\nc\nd\nE\n");
    assert_eq!(read(&root.join("feature")), "new\n");
    assert_eq!(git(&root, &["rev-parse", "HEAD"]), head, "HEAD untouched");
    assert_eq!(
        std::fs::read(root.join(".git/index")).unwrap(),
        index,
        "index untouched"
    );
    // The change is in the tree, so the worktrees that carried it are closed: none outlives
    // the conversation, and nothing of theirs is lost.
    assert!(!worktree.exists(), "the helper's worktree is closed");
    assert!(!trial.worktree.exists(), "and so is the trial's");
    assert!(
        Workspaces::open(&root, &session)
            .unwrap()
            .workspaces
            .is_empty()
    );
    assert_eq!(
        git(&root, &["worktree", "list"]).lines().count(),
        1,
        "no worktree left registered"
    );
}

#[test]
fn a_conflict_is_reported_with_markers_in_the_trial_worktree_and_resolved_there() {
    let repo = repository();
    let root = repo.path().canonicalize().unwrap();
    let session = directory(&root, "one");
    let mut work = Workspaces::open(&root, &session).unwrap();
    let worktree = work.create("backend").unwrap();
    std::fs::write(worktree.join("tracked"), "a\nb\nTHEIRS\nd\ne\n").unwrap();
    std::fs::write(root.join("tracked"), "a\nb\nOURS\nd\ne\n").unwrap();

    let backend = work.find("backend").unwrap();
    let mut trial = merge::trial(&root, backend, &session, 1).unwrap();
    assert_eq!(trial.state, State::Conflicts);
    assert_eq!(trial.conflicts, ["tracked"]);
    let marked = read(&trial.root.join("tracked"));
    assert!(
        marked.contains("<<<<<<<") && marked.contains(">>>>>>>"),
        "{marked}"
    );
    assert_eq!(
        read(&root.join("tracked")),
        "a\nb\nOURS\nd\ne\n",
        "markers never reach the user's tree"
    );
    assert!(
        merge::apply(&root, &session, &mut trial).is_err(),
        "a conflicted trial cannot be applied"
    );

    // A resolution seat works in the trial's worktree; checking again finds it clean.
    std::fs::write(trial.root.join("tracked"), "a\nb\nBOTH\nd\ne\n").unwrap();
    merge::rechecked(&mut trial).unwrap();
    assert_eq!(trial.state, State::Clean, "{trial:?}");
    assert_eq!(trial.files, ["tracked"]);
    // Markers left behind keep a trial conflicted.
    let mut again = merge::trial(&root, work.find("backend").unwrap(), &session, 2).unwrap();
    assert_eq!(again.state, State::Conflicts, "{again:?}");
    merge::rechecked(&mut again).unwrap();
    assert_eq!(again.state, State::Conflicts);

    merge::apply(&root, &session, &mut trial).unwrap();
    assert_eq!(read(&root.join("tracked")), "a\nb\nBOTH\nd\ne\n");
    assert!(!worktree.exists());
    // A trial left behind goes with the conversation's sweep.
    assert!(again.worktree.exists());
    assert_eq!(merge::sweep_trials(&root, &session).unwrap(), 1);
    assert!(!again.worktree.exists());
}

#[test]
fn an_apply_refuses_a_tree_that_moved_since_the_trial_and_writes_nothing() {
    let repo = repository();
    let root = repo.path().canonicalize().unwrap();
    let session = directory(&root, "one");
    let mut work = Workspaces::open(&root, &session).unwrap();
    let worktree = work.create("backend").unwrap();
    std::fs::write(worktree.join("feature"), "new\n").unwrap();
    let mut trial = merge::trial(&root, work.find("backend").unwrap(), &session, 1).unwrap();
    assert_eq!(trial.state, State::Clean);

    std::fs::write(root.join("tracked"), "moved\n").unwrap();
    let error = merge::apply(&root, &session, &mut trial).unwrap_err();
    assert!(
        error.to_string().contains("changed since the trial"),
        "{error}"
    );
    assert_eq!(trial.state, State::Stale);
    assert!(!root.join("feature").exists(), "nothing was written");
    assert_eq!(read(&root.join("tracked")), "moved\n");

    // A trial on the tree as it is now applies.
    let work = Workspaces::open(&root, &session).unwrap();
    let mut fresh = merge::trial(&root, work.find("backend").unwrap(), &session, 2).unwrap();
    assert_eq!(fresh.state, State::Clean);
    merge::apply(&root, &session, &mut fresh).unwrap();
    assert_eq!(read(&root.join("feature")), "new\n");
    assert_eq!(read(&root.join("tracked")), "moved\n");
}

/// The conversation's sweep closes a trial left behind and the helper's worktree without
/// writing anything into the user's tree: only an apply writes there.
#[test]
fn the_sweep_closes_trials_and_worktrees_without_touching_the_tree() {
    let repo = repository();
    let root = repo.path().canonicalize().unwrap();
    let session = directory(&root, "one");
    let mut work = Workspaces::open(&root, &session).unwrap();
    let worktree = work.create("backend").unwrap();
    std::fs::write(worktree.join("feature"), "new\n").unwrap();
    std::fs::write(worktree.join("tracked"), "a\nb\nc\nd\nE\n").unwrap();
    let trial = merge::trial(&root, work.find("backend").unwrap(), &session, 1).unwrap();
    assert_eq!(trial.state, State::Clean);
    let before = git(&root, &["status", "--porcelain"]);
    let closed = work
        .close_all(&orochi::workspaces::Tools::default())
        .unwrap();
    assert_eq!(closed.len(), 1);
    assert_eq!(merge::sweep_trials(&root, &session).unwrap(), 1);
    assert_eq!(
        git(&root, &["status", "--porcelain"]),
        before,
        "the sweep wrote into the tree"
    );
    assert!(!root.join("feature").exists());
    assert_eq!(read(&root.join("tracked")), "a\nb\nc\nd\ne\n");
    assert_eq!(git(&root, &["worktree", "list"]).lines().count(), 1);
}
