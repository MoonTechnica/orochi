//! A library the window ships is a file in `dist/vendor`, and the list beside it is what it was
//! when it was read from its package. A file edited by hand, or replaced, fails here.
use sha2::{Digest, Sha256};

#[test]
fn every_vendored_file_is_the_one_that_was_recorded() {
    let vendor = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../dist/vendor");
    let mut checked = 0;
    for library in std::fs::read_dir(&vendor).unwrap().flatten() {
        let sums = std::fs::read_to_string(library.path().join("SHA256SUMS"))
            .unwrap_or_else(|_| panic!("{} has no SHA256SUMS", library.path().display()));
        for line in sums.lines().filter(|l| !l.trim().is_empty()) {
            let (hash, name) = line.split_once("  ").expect("`hash  name`");
            let bytes = std::fs::read(library.path().join(name)).unwrap();
            assert_eq!(
                format!("{:x}", Sha256::digest(&bytes)),
                hash,
                "{name} was changed"
            );
            checked += 1;
        }
        assert!(
            library.path().join("LICENSE").exists(),
            "{} has no licence",
            library.path().display()
        );
    }
    assert!(
        checked >= 20,
        "the terminal's four files the PDF reader's three the Word reader's four the spreadsheet reader's two the presentation reader's four and the model viewer's three"
    );
}
