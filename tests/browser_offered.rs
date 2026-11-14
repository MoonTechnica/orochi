//! Orochi's own browser, once the process offers it, counts as every agent's ability to browse
//! (`AgentConfig::abilities`), except where the tools cannot reach the agent. Its own file: the
//! offer is process-wide, and no other test should find the browser on.
#![cfg(unix)]
use orochi::{
    browser,
    config::{AgentConfig, BrowserConfig},
    types::Provider,
};

fn agent() -> AgentConfig {
    AgentConfig::preset(
        "claude",
        Provider::new("anthropic").unwrap(),
        "claude-agent-acp",
        &[],
    )
}

#[test]
fn once_offered_the_browser_is_every_agents_but_not_a_sandboxed_or_routing_only_ones() {
    let data = tempfile::tempdir().unwrap();
    assert!(
        !browser::active(),
        "nothing is offered before it is registered"
    );
    assert!(!agent().abilities().browser);

    browser::register(
        data.path(),
        std::path::PathBuf::from("/bin/true"),
        &BrowserConfig::default(),
    )
    .unwrap();
    assert!(browser::active());
    assert!(
        agent().abilities().browser,
        "a task that needs a browser no longer sends the agent away"
    );
    let mut sandboxed = agent();
    sandboxed.sandboxed = true;
    assert!(
        !sandboxed.abilities().browser,
        "its tools run on this machine, not in the VM"
    );
    let mut adviser = agent();
    adviser.routing_only = true;
    assert!(!adviser.abilities().browser, "an adviser is given no tools");
    let mut claimed = agent();
    claimed.sandboxed = true;
    claimed.browser = true;
    assert!(
        claimed.abilities().browser,
        "what the user said still stands"
    );
}

#[test]
fn the_browser_is_on_unless_the_person_turned_it_off() {
    assert!(BrowserConfig::default().enabled);
}
