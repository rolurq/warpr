//! Lives in its own test binary because it mutates the process environment, which is not safe
//! to do while other tests run threads in the same process.

use std::path::Path;

use herdr_client::{
    ConnectionStatus, HerdrCli, SessionName, SessionTarget, present_session,
};
use herdr_fake::{FakeHerdr, SessionScript};

fn labels(workspaces: &herdr_client::PresentedSession) -> Vec<&str> {
    workspaces
        .workspaces
        .iter()
        .map(|workspace| workspace.label.as_str())
        .collect()
}

#[test]
fn a_session_inherited_from_a_launching_herdr_pane_is_never_used() {
    let mut fake = FakeHerdr::new();
    let launching_pane_session =
        fake.start_session("outer", SessionScript::new().workspace("outer-workspace"));
    let _chosen = fake.start_session("inner", SessionScript::new().workspace("inner-workspace"));
    fake.script_command(&["session", "list", "--json"], "[]\n", 0);
    let chosen = SessionTarget::new(&fake.config_dir(), SessionName::new("inner").unwrap());

    // SAFETY: this is the only test in this binary, so nothing reads the environment concurrently.
    unsafe {
        std::env::set_var("HERDR_SOCKET_PATH", launching_pane_session.socket_path());
        std::env::set_var("HERDR_SESSION", "outer");
        std::env::set_var("HERDR_PANE_ID", "w9:p9");
        std::env::set_var("HERDR_TAB_ID", "w9:t9");
        std::env::set_var("HERDR_WORKSPACE_ID", "w9");
    }

    let presented = present_session(&chosen);
    assert_eq!(presented.connection, ConnectionStatus::Connected);
    assert_eq!(labels(&presented), ["inner-workspace"]);
    assert_eq!(launching_pane_session.requests_served(), 0);

    let output = HerdrCli::new(fake.binary_path())
        .run(&chosen, &["session", "list", "--json"])
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&output.stdout), "[]\n");
    let invocations = fake.invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(
        invocations[0].env("HERDR_SOCKET_PATH").map(Path::new),
        Some(chosen.socket_path())
    );
    assert_eq!(invocations[0].env("HERDR_SESSION"), Some("inner"));
    assert_eq!(invocations[0].env("HERDR_PANE_ID"), None);
    assert_eq!(invocations[0].env("HERDR_TAB_ID"), None);
    assert_eq!(invocations[0].env("HERDR_WORKSPACE_ID"), None);
}
