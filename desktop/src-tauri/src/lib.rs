//! A window onto the conversations Orochi records.
//!
//! The app is a view over SQLite: the core writes rows, this reads them, and the few things
//! the window changes are rows too. `view` is that whole contract; `main.rs` is a shell that
//! hands it to a webview.
pub mod view;
