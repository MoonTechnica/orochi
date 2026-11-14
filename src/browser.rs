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
mod server;
#[cfg(unix)]
pub use server::*;
