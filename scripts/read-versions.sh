# Sourced, from the repository root. Sets `cli`, `desktop` and `bundle` to the version each of the
# three files names. They are meant to agree; `scripts/version.sh` sets them and the release
# workflow refuses a tag that does not match.
cli=$(sed -n '/^\[package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
desktop=$(sed -n '/^\[package\]/,/^\[/s/^version = "\(.*\)"/\1/p' desktop/src-tauri/Cargo.toml | head -n 1)
bundle=$(sed -n 's/.*"version":[ 	]*"\([^"]*\)".*/\1/p' desktop/src-tauri/tauri.conf.json | head -n 1)
