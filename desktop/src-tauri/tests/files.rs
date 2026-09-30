//! The Files pane: the thread's working tree, read from the disk and never from the store
//! (`docs/files-pane-design.md`). Run against a real `git init`, because git's own answers are
//! what the pane reads.
use orochi_desktop::files::{self, Kind, Status};
use orochi_desktop::view::Client;
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

fn write(path: &Path, text: impl AsRef<[u8]>) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A repository with every state the pane draws, and the paths it must refuse.
struct Tree {
    _dir: tempfile::TempDir,
    data: std::path::PathBuf,
    repo: std::path::PathBuf,
    outside: std::path::PathBuf,
}

fn tree() -> Tree {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let repo = dir.path().join("repo");
    let outside = dir.path().join("secret.txt");
    write(&outside, "not yours\n");
    write(&repo.join("README.md"), "# Title\n\nbody\n");
    write(&repo.join("Cargo.toml"), "[package]\n");
    write(&repo.join(".gitignore"), "target/\n");
    write(&repo.join("src/chat/term.rs"), "fn a() {}\n");
    write(&repo.join("src/chat/host.rs"), "fn b() {}\n");
    write(&repo.join("src/lib.rs"), "pub mod chat;\n");
    write(&repo.join("docs/gone.md"), "soon gone\n");
    git(&repo, &["init", "-q"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    write(&repo.join("src/chat/term.rs"), "fn a() { changed() }\n");
    write(&repo.join("new.txt"), "untracked\n");
    write(&repo.join("target/debug/out"), "built\n");
    std::fs::remove_file(repo.join("docs/gone.md")).unwrap();
    write(&repo.join("blob.bin"), b"ab\0cd");
    write(&repo.join("big.txt"), "x".repeat(600 * 1024));
    write(
        &repo.join("pixel.png"),
        [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a],
    );
    std::os::unix::fs::symlink(&outside, repo.join("link-out")).unwrap();
    std::os::unix::fs::symlink(repo.join("src"), repo.join("link-in")).unwrap();
    Tree {
        data,
        repo,
        outside,
        _dir: dir,
    }
}

fn open(tree: &Tree) -> (Client, String) {
    std::fs::create_dir_all(&tree.data).unwrap();
    let client = Client::open(&tree.data).unwrap();
    let thread = client.new_thread(&tree.repo).unwrap();
    (client, thread)
}

#[test]
fn a_folder_lists_directories_first_with_the_git_state_of_each_row() {
    let tree = tree();
    let (mut client, thread) = open(&tree);
    let root = client.tree_list(&thread, "").unwrap();
    let names: Vec<&str> = root.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "docs",
            "src",
            "target",
            ".gitignore",
            "big.txt",
            "blob.bin",
            "Cargo.toml",
            "link-in",
            "link-out",
            "new.txt",
            "pixel.png",
            "README.md",
        ],
        "directories first, then the rest, case-insensitively; .git is never listed"
    );
    let row = |name: &str| root.iter().find(|e| e.name == name).unwrap();
    assert_eq!(
        row("src").within,
        Some(Status::Modified),
        "a folder carries what is under it"
    );
    assert_eq!(row("src").status, None);
    assert_eq!(row("target").status, Some(Status::Ignored));
    assert_eq!(row("new.txt").status, Some(Status::Untracked));
    assert_eq!(row("docs").within, Some(Status::Deleted));
    assert_eq!(
        row("link-out").kind,
        Kind::Symlink,
        "a link is listed as one"
    );
    assert_eq!(
        row("link-in").kind,
        Kind::Symlink,
        "and never expanded as a folder"
    );
    assert_eq!(row("README.md").status, None);
    assert_eq!(row("README.md").size, Some(14));

    let chat = client.tree_list(&thread, "src/chat").unwrap();
    let term = chat.iter().find(|e| e.name == "term.rs").unwrap();
    assert_eq!(term.path, "src/chat/term.rs");
    assert_eq!(term.status, Some(Status::Modified));
    assert_eq!(
        client.tree_list(&thread, "src").unwrap()[0].within,
        Some(Status::Modified),
        "every ancestor carries the fold"
    );

    let docs = client.tree_list(&thread, "docs").unwrap();
    assert_eq!(docs.len(), 1, "a deleted file is still shown where it was");
    assert!(docs[0].missing);
    assert_eq!(docs[0].status, Some(Status::Deleted));

    let ignored = client.tree_list(&thread, "target/debug").unwrap();
    assert_eq!(
        ignored[0].status,
        Some(Status::Ignored),
        "what is inside an ignored folder is ignored with it"
    );
}

#[test]
fn nothing_outside_the_thread_folder_can_be_named() {
    let tree = tree();
    let (mut client, thread) = open(&tree);
    for path in [
        tree.outside.to_str().unwrap(),
        "/etc/passwd",
        "../secret.txt",
        "src/../../secret.txt",
        ".git/config",
        "src/.git",
        "link-out",
    ] {
        assert!(
            client.tree_read(&thread, path).is_err(),
            "{path} is refused"
        );
        assert!(
            client.tree_list(&thread, path).is_err(),
            "{path} is refused"
        );
        assert!(
            client.open_path(&thread, path, true).is_err(),
            "{path} is refused"
        );
    }
    assert_eq!(
        client.tree_read(&thread, "link-in/lib.rs").unwrap().text,
        "pub mod chat;\n",
        "a link that stays inside may be followed"
    );
}

#[test]
fn a_file_is_read_bounded_and_said_to_be_what_it_is() {
    let tree = tree();
    let (mut client, thread) = open(&tree);
    let text = client.tree_read(&thread, "src/chat/term.rs").unwrap();
    assert_eq!(text.text, "fn a() { changed() }\n");
    assert_eq!(text.lines, 1);
    assert_eq!(text.status, Some(Status::Modified));
    assert!(!text.binary && !text.truncated && text.image.is_none());

    let big = client.tree_read(&thread, "big.txt").unwrap();
    assert!(big.truncated);
    assert_eq!(big.text.len() as u64, files::READ_LIMIT);
    assert_eq!(big.bytes, 600 * 1024, "and says how big the whole file is");

    let blob = client.tree_read(&thread, "blob.bin").unwrap();
    assert!(blob.binary);
    assert!(
        blob.text.is_empty(),
        "a binary file's bytes are not sent as text"
    );

    let png = client.tree_read(&thread, "pixel.png").unwrap();
    assert!(png.image.unwrap().starts_with("data:image/png;base64,"));
    assert!(png.text.is_empty());

    assert!(
        client.tree_read(&thread, "src").is_err(),
        "a folder is not a file"
    );
}

#[test]
fn files_are_found_by_name_and_lines_by_content() {
    let tree = tree();
    let (client, thread) = open(&tree);
    let hits = client
        .tree_find(&thread, "term", "name", 50)
        .unwrap()
        .unwrap();
    assert_eq!(
        hits[0].path, "src/chat/term.rs",
        "a name that contains it comes first"
    );
    let hits = client
        .tree_find(&thread, "sct", "name", 50)
        .unwrap()
        .unwrap();
    assert!(
        hits.iter().any(|h| h.path == "src/chat/term.rs"),
        "letters in order find a file too"
    );
    assert_eq!(
        client
            .tree_find(&thread, "s", "name", 50)
            .unwrap()
            .unwrap()
            .len(),
        0,
        "one letter asks nothing"
    );
    assert_eq!(
        client
            .tree_find(&thread, ".rs", "name", 2)
            .unwrap()
            .unwrap()
            .len(),
        2,
        "and the answer stops at the limit"
    );

    let hits = client
        .tree_find(&thread, "CHANGED", "content", 50)
        .unwrap()
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].path, "src/chat/term.rs");
    assert_eq!(hits[0].line, Some(1));
    assert_eq!(hits[0].text.as_deref(), Some("fn a() { changed() }"));
    assert!(
        client
            .tree_find(&thread, "untracked", "content", 50)
            .unwrap()
            .unwrap()
            .iter()
            .any(|h| h.path == "new.txt"),
        "a file git does not track yet is searched too"
    );
    assert!(client.tree_find(&thread, "x", "sideways", 5).is_err());
}

#[test]
fn outside_a_repository_names_are_still_found_and_content_search_says_it_cannot() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let plain = dir.path().join("plain");
    write(&plain.join("notes/today.md"), "hello\n");
    write(&plain.join("node_modules/pkg/today.js"), "skip\n");
    std::fs::create_dir_all(&data).unwrap();
    let mut client = Client::open(&data).unwrap();
    let thread = client.new_thread(&plain).unwrap();
    let hits = client
        .tree_find(&thread, "today", "name", 50)
        .unwrap()
        .unwrap();
    assert_eq!(
        hits.iter().map(|h| h.path.as_str()).collect::<Vec<_>>(),
        vec!["notes/today.md"],
        "a walk skips what nobody browses"
    );
    assert!(
        client
            .tree_find(&thread, "hello", "content", 50)
            .unwrap()
            .is_none()
    );
    let root = client.tree_list(&thread, "").unwrap();
    assert!(
        root.iter().all(|e| e.status.is_none()),
        "no repository, no git state"
    );
    assert!(!client.tree_refresh(&thread).unwrap().repository);
}

#[test]
fn the_state_is_read_once_and_again_only_when_asked() {
    let tree = tree();
    let (mut client, thread) = open(&tree);
    let before = client.tree_list(&thread, "").unwrap();
    assert_eq!(
        before
            .iter()
            .find(|e| e.name == "README.md")
            .unwrap()
            .status,
        None
    );
    write(&tree.repo.join("README.md"), "# Title\n\nedited\n");
    assert_eq!(
        client
            .tree_list(&thread, "")
            .unwrap()
            .iter()
            .find(|e| e.name == "README.md")
            .unwrap()
            .status,
        None,
        "one git status serves every folder until something says the tree moved"
    );
    let state = client.tree_refresh(&thread).unwrap();
    assert!(state.repository && !state.partial);
    assert_eq!(
        client
            .tree_list(&thread, "")
            .unwrap()
            .iter()
            .find(|e| e.name == "README.md")
            .unwrap()
            .status,
        Some(Status::Modified)
    );
}

#[test]
fn a_repository_config_cannot_make_the_pane_run_anything() {
    let tree = tree();
    let marker = tree.repo.parent().unwrap().join("ran");
    let hook = tree.repo.parent().unwrap().join("hook.sh");
    write(&hook, format!("#!/bin/sh\ntouch {}\n", marker.display()));
    Command::new("chmod").arg("+x").arg(&hook).status().unwrap();
    git(
        &tree.repo,
        &["config", "core.fsmonitor", hook.to_str().unwrap()],
    );
    let (mut client, thread) = open(&tree);
    client.tree_refresh(&thread).unwrap();
    client.tree_list(&thread, "").unwrap();
    client.tree_find(&thread, "changed", "content", 5).unwrap();
    assert!(
        !marker.exists(),
        "the repository's fsmonitor was not started"
    );
}

/// The page's tests are driven by `desktop/tests/files.json`. A field named there and never
/// sent would let the page read it for ever with nothing failing, so each recorded shape is
/// compared with what these commands actually serialize.
#[test]
fn the_recorded_pane_output_has_the_fields_the_commands_send() {
    let tree = tree();
    let (mut client, thread) = open(&tree);
    let recorded: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/files.json"))
            .unwrap(),
    )
    .unwrap();
    let keys = |value: &serde_json::Value| {
        let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    };
    let sent = |value: serde_json::Value| keys(&value);
    let entry = serde_json::to_value(&client.tree_list(&thread, "").unwrap()[0]).unwrap();
    let file = serde_json::to_value(client.tree_read(&thread, "README.md").unwrap()).unwrap();
    let hit = serde_json::to_value(
        &client
            .tree_find(&thread, "changed", "content", 5)
            .unwrap()
            .unwrap()[0],
    )
    .unwrap();
    let state = serde_json::to_value(client.tree_refresh(&thread).unwrap()).unwrap();
    for (name, shape) in [
        ("root", &entry),
        ("src", &entry),
        ("src/chat", &entry),
        ("name", &hit),
        ("content", &hit),
    ] {
        for row in recorded[name].as_array().unwrap() {
            assert_eq!(keys(row), sent(shape.clone()), "{name}");
        }
    }
    for name in ["text", "markdown", "binary"] {
        assert_eq!(keys(&recorded[name]), sent(file.clone()), "{name}");
    }
    assert_eq!(keys(&recorded["refresh"]), sent(state), "refresh");
}
