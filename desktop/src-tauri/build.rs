fn main() {
    // The window is built into the binary, and Cargo watches only Rust. Without this, editing
    // the page and running `cargo build` printed "Finished" and changed nothing: two fixes in a
    // row reached a window that was still running the old page, and looked like bad fixes.
    println!("cargo:rerun-if-changed=../dist");
    for entry in std::fs::read_dir("../dist").into_iter().flatten().flatten() {
        println!("cargo:rerun-if-changed={}", entry.path().display());
    }
    tauri_build::build();
}
