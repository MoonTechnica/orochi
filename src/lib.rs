pub mod acp;
pub mod activity;
pub mod agents;
pub mod benchmark;
pub mod chat;
pub mod cli;
pub mod collaboration;
pub mod config;
pub mod context;
pub mod discovery;
pub mod effort;
pub mod evaluator;
pub mod gateway;
pub mod interrupt;
pub mod learning;
pub mod mailbox;
pub mod mcp;
pub mod memory;
pub mod pane;
pub mod policy;
#[cfg(unix)]
pub mod process;
/// Off Unix nothing is supervised, but the mailbox and the activity store still record who owns
/// a peer or a host. Without a start time a reused pid cannot be told apart, so an owner is
/// taken to be alive.
#[cfg(not(unix))]
pub mod process {
    pub fn owner_identity() -> anyhow::Result<(i64, i64)> {
        Ok((i64::from(std::process::id()), 0))
    }

    pub fn owner_alive(_pid: i64, _start: i64) -> bool {
        true
    }
}
pub mod quota_sources;
pub mod router;
pub mod sandbox;
pub mod scheduler;
pub mod storage;
pub mod types;

pub mod workspaces;
