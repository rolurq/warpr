use std::ffi::OsString;
use std::path::Path;

use herdr_fake::{FakeHerdr, HerdrStatus, SessionScript};

use super::*;
use crate::session::config_dir_from;

fn target(fake: &FakeHerdr, name: &str) -> SessionTarget {
    SessionTarget::new(&fake.config_dir(), SessionName::new(name).unwrap())
}

fn fake_with(status: HerdrStatus) -> FakeHerdr {
    let mut fake = FakeHerdr::new();
    fake.script_status(&status);
    fake
}

fn compatible_fake() -> FakeHerdr {
    fake_with(HerdrStatus::compatible())
}

fn present(fake: &FakeHerdr, session: &str) -> PresentedSession {
    present_session(&HerdrCli::new(fake.binary_path()), &target(fake, session))
}

fn labels(session: &PresentedSession) -> Vec<&str> {
    session
        .workspaces
        .iter()
        .map(|workspace| workspace.label.as_str())
        .collect()
}

#[test]
fn presents_workspaces_with_herdr_labels_in_herdr_order() {
    let fake = compatible_fake();
    let _session = fake.start_session(
        "default",
        SessionScript::new()
            .workspace("api")
            .workspace("web")
            .workspace("docs"),
    );

    let presented = present(&fake, "default");

    assert_eq!(presented.connection, ConnectionStatus::Connected);
    assert_eq!(labels(&presented), ["api", "web", "docs"]);
}

#[test]
fn presents_the_workspace_herdr_has_focused() {
    let fake = compatible_fake();
    let _session = fake.start_session(
        "default",
        SessionScript::new()
            .workspace("api")
            .workspace("web")
            .workspace("docs")
            .focused("web"),
    );

    let presented = present(&fake, "default");

    assert_eq!(presented.focused_workspace_index(), Some(1));
    assert_eq!(
        presented.focused_workspace,
        Some(presented.workspaces[1].id.clone())
    );
}

#[test]
fn a_session_without_workspaces_presents_none_and_no_focus() {
    let fake = compatible_fake();
    let _session = fake.start_session("default", SessionScript::new());

    let presented = present(&fake, "default");

    assert_eq!(presented.connection, ConnectionStatus::Connected);
    assert!(presented.workspaces.is_empty());
    assert_eq!(presented.focused_workspace_index(), None);
}

#[test]
fn only_the_chosen_sessions_socket_is_used() {
    let fake = compatible_fake();
    let default = fake.start_session("default", SessionScript::new().workspace("in-default"));
    let _work = fake.start_session("work", SessionScript::new().workspace("in-work"));

    let presented = present(&fake, "work");

    assert_eq!(labels(&presented), ["in-work"]);
    assert_eq!(default.requests_served(), 0);
}

#[test]
fn an_unreachable_socket_is_presented_as_a_failed_connection() {
    let fake = fake_with(HerdrStatus::server_not_running());

    let presented = present(&fake, "stopped");

    assert!(matches!(
        presented.connection,
        ConnectionStatus::Failed(ConnectionFailure::SocketUnreachable { .. })
    ));
    assert!(presented.workspaces.is_empty());
    assert_eq!(presented.focused_workspace, None);
}

#[test]
fn the_default_session_socket_sits_directly_in_the_config_dir() {
    let target = SessionTarget::new(Path::new("/cfg/herdr"), SessionName::default_session());

    assert_eq!(target.socket_path(), Path::new("/cfg/herdr/herdr.sock"));
}

#[test]
fn a_named_session_socket_sits_under_sessions() {
    let target = SessionTarget::new(Path::new("/cfg/herdr"), SessionName::new("work").unwrap());

    assert_eq!(
        target.socket_path(),
        Path::new("/cfg/herdr/sessions/work/herdr.sock")
    );
}

#[test]
fn session_names_cannot_escape_the_sessions_directory() {
    for name in ["", ".", "..", "a/b", "../x", "a\\b"] {
        assert!(
            SessionName::new(name).is_err(),
            "{name:?} should be rejected"
        );
    }
}

#[test]
fn config_dir_prefers_xdg_config_home_and_falls_back_to_dot_config() {
    let os = |value: &str| Some(OsString::from(value));

    assert_eq!(
        config_dir_from(os("/xdg"), os("/home/u")),
        Some("/xdg/herdr".into())
    );
    assert_eq!(
        config_dir_from(None, os("/home/u")),
        Some("/home/u/.config/herdr".into())
    );
    assert_eq!(
        config_dir_from(os(""), os("/home/u")),
        Some("/home/u/.config/herdr".into())
    );
    assert_eq!(config_dir_from(None, None), None);
}

#[test]
fn an_invalid_session_name_is_reported_as_unresolvable() {
    let failure = SessionTarget::for_current_user("../escape").unwrap_err();

    assert!(matches!(
        failure,
        ConnectionFailure::SessionUnresolvable { .. }
    ));
}
