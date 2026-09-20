//! The window's rendering, driven from `cargo test` so it is covered by the same command as
//! everything else. The scenarios live in `desktop/tests/ui.test.mjs`, which runs the real
//! `dist/app.js` against view output recorded from a real run.
use std::process::Command;

#[test]
fn the_window_draws_what_the_views_return() {
    let desktop = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("desktop directory");
    let status = Command::new("node")
        .args(["--test", "tests/ui.test.mjs"])
        .current_dir(desktop)
        .status()
        .expect("node is needed to test the window, as python3 is to test the terminal");
    assert!(status.success());
}
