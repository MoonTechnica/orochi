//! A browser the person and the agents share (`docs/conversation-tabs-design.md` §3.5).
//!
//! One Chrome per machine, run headless over a debugging *pipe* by `orochi --internal-browser`
//! (the broker), which owns it, applies the host policy and speaks the protocol. An agent
//! session gets `orochi --internal-browser-mcp`, a small stdio MCP server that forwards each
//! tool call to the broker over a Unix socket; the window will draw the same browser as a
//! viewer. Nothing outside this machine is contacted except the addresses the policy lets a
//! page open.
pub mod policy;

#[cfg(unix)]
pub mod broker;
#[cfg(unix)]
pub mod cdp;
#[cfg(unix)]
pub mod engine;
#[cfg(unix)]
pub mod hub;
#[cfg(unix)]
pub mod preview;
#[cfg(unix)]
pub mod remote;
#[cfg(unix)]
mod server;
#[cfg(unix)]
pub use server::*;

/// Whether the sessions this process starts are offered Orochi's browser.
pub fn active() -> bool {
    #[cfg(unix)]
    {
        server::is_registered()
    }
    #[cfg(not(unix))]
    {
        false
    }
}
