use herdr_fake::{FakeHerdr, HerdrStatus, Invocation, SessionScript};

use super::*;
use crate::{ConnectionStatus, Refusal};

fn target(fake: &FakeHerdr, name: &str) -> SessionTarget {
    SessionTarget::new(&fake.config_dir(), SessionName::new(name).unwrap())
}

fn cli(fake: &FakeHerdr) -> HerdrCli {
    HerdrCli::new(fake.binary_path())
}

fn script_compatible(fake: &mut FakeHerdr) {
    fake.script_status(&HerdrStatus::compatible());
}

fn labels(session: &PresentedSession) -> Vec<&str> {
    session
        .workspaces
        .iter()
        .map(|workspace| workspace.label.as_str())
        .collect()
}

fn server_starts(fake: &FakeHerdr) -> Vec<Invocation> {
    fake.invocations()
        .into_iter()
        .filter(|invocation| invocation.args.last().is_some_and(|arg| arg == "server"))
        .collect()
}

#[test]
fn lists_every_session_with_its_state_in_herdrs_order() {
    let mut fake = FakeHerdr::new();
    fake.script_session_list(&[("default", true), ("api", false), ("web", true)]);

    let sessions = list_sessions(&cli(&fake)).unwrap();

    let listed: Vec<_> = sessions
        .iter()
        .map(|session| (session.name.as_str(), session.state))
        .collect();
    assert_eq!(
        listed,
        [
            ("default", SessionState::Running),
            ("api", SessionState::Stopped),
            ("web", SessionState::Running),
        ]
    );
}

#[test]
fn a_listing_that_fails_is_reported_with_herdrs_message() {
    let mut fake = FakeHerdr::new();
    fake.script_command(&["session", "list", "--json"], "", 3);

    let failure = list_sessions(&cli(&fake)).unwrap_err();

    assert!(matches!(
        failure,
        ConnectionFailure::SessionListUnavailable { .. }
    ));
}

#[test]
fn an_unreadable_listing_is_reported_instead_of_read_as_empty() {
    let mut fake = FakeHerdr::new();
    fake.script_command(&["session", "list", "--json"], "not json\n", 0);

    let failure = list_sessions(&cli(&fake)).unwrap_err();

    assert!(matches!(
        failure,
        ConnectionFailure::SessionListUnavailable { .. }
    ));
}

#[test]
fn a_missing_herdr_executable_is_reported_as_an_unavailable_listing() {
    let failure = list_sessions(&HerdrCli::new("/nonexistent/herdr")).unwrap_err();

    assert!(matches!(
        failure,
        ConnectionFailure::SessionListUnavailable { .. }
    ));
}

#[test]
fn opening_a_running_session_connects_without_starting_a_server() {
    let mut fake = FakeHerdr::new();
    script_compatible(&mut fake);
    let _work = fake.start_session("work", SessionScript::new().workspace("in-work"));

    let presented = open_session(&cli(&fake), &target(&fake, "work"), SessionState::Running);

    assert_eq!(presented.connection, ConnectionStatus::Connected);
    assert_eq!(labels(&presented), ["in-work"]);
    assert!(server_starts(&fake).is_empty());
}

#[test]
fn opening_a_stopped_session_starts_its_headless_server_then_connects() {
    let mut fake = FakeHerdr::new();
    script_compatible(&mut fake);
    fake.script_server_start("work", SessionScript::new().workspace("in-work"));
    let work = target(&fake, "work");

    let presented = open_session(&cli(&fake), &work, SessionState::Stopped);

    assert_eq!(presented.connection, ConnectionStatus::Connected);
    assert_eq!(labels(&presented), ["in-work"]);
    let starts = server_starts(&fake);
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0].args, ["--session", "work", "server"]);
    assert_eq!(
        starts[0].env("HERDR_SOCKET_PATH").map(std::path::Path::new),
        Some(work.socket_path())
    );
    assert_eq!(starts[0].env("HERDR_SESSION"), Some("work"));
}

#[test]
fn a_server_that_fails_to_start_is_presented_as_a_failed_connection() {
    let mut fake = FakeHerdr::new();
    fake.script_command(&["--session", "work", "server"], "", 1);

    let presented = open_session(&cli(&fake), &target(&fake, "work"), SessionState::Stopped);

    assert!(matches!(
        presented.connection,
        ConnectionStatus::Failed(ConnectionFailure::ServerStartFailed { .. })
    ));
    assert!(presented.workspaces.is_empty());
}

#[test]
fn a_missing_herdr_executable_fails_the_start_of_a_stopped_session() {
    let fake = FakeHerdr::new();

    let presented = open_session(
        &HerdrCli::new("/nonexistent/herdr"),
        &target(&fake, "work"),
        SessionState::Stopped,
    );

    assert!(matches!(
        presented.connection,
        ConnectionStatus::Failed(ConnectionFailure::ServerStartFailed { .. })
    ));
}

#[test]
fn launching_starts_the_default_session_when_none_is_running() {
    let mut fake = FakeHerdr::new();
    script_compatible(&mut fake);
    fake.script_session_list(&[("default", false), ("work", true)]);
    fake.script_server_start("default", SessionScript::new().workspace("main"));

    let presented = launch_session(&cli(&fake), &target(&fake, "default"));

    assert_eq!(presented.connection, ConnectionStatus::Connected);
    assert_eq!(labels(&presented), ["main"]);
    let starts = server_starts(&fake);
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0].args, ["server"]);
    assert_eq!(starts[0].env("HERDR_SESSION"), Some("default"));
}

#[test]
fn launching_leaves_a_running_default_session_alone() {
    let mut fake = FakeHerdr::new();
    script_compatible(&mut fake);
    fake.script_session_list(&[("default", true)]);
    let _default = fake.start_session("default", SessionScript::new().workspace("main"));

    let presented = launch_session(&cli(&fake), &target(&fake, "default"));

    assert_eq!(labels(&presented), ["main"]);
    assert!(server_starts(&fake).is_empty());
}

#[test]
fn launching_a_named_stopped_session_starts_it() {
    let mut fake = FakeHerdr::new();
    script_compatible(&mut fake);
    fake.script_session_list(&[("default", true), ("work", false)]);
    fake.script_server_start("work", SessionScript::new().workspace("in-work"));

    let presented = launch_session(&cli(&fake), &target(&fake, "work"));

    assert_eq!(labels(&presented), ["in-work"]);
    assert_eq!(server_starts(&fake).len(), 1);
}

#[test]
fn launching_a_session_herdr_does_not_list_reports_it_instead_of_creating_it() {
    let mut fake = FakeHerdr::new();
    script_compatible(&mut fake);
    fake.script_session_list(&[("default", true)]);

    let presented = launch_session(&cli(&fake), &target(&fake, "typo"));

    assert!(matches!(
        presented.connection,
        ConnectionStatus::Failed(ConnectionFailure::SessionUnresolvable { .. })
    ));
    assert!(server_starts(&fake).is_empty());
}

#[test]
fn launching_without_a_listing_reports_the_failure() {
    let mut fake = FakeHerdr::new();
    script_compatible(&mut fake);
    fake.script_command(&["session", "list", "--json"], "", 3);

    let presented = launch_session(&cli(&fake), &target(&fake, "default"));

    assert!(matches!(
        presented.connection,
        ConnectionStatus::Failed(ConnectionFailure::SessionListUnavailable { .. })
    ));
}

#[test]
fn launching_without_a_herdr_executable_is_refused_rather_than_failed() {
    let fake = FakeHerdr::new();

    let presented = launch_session(
        &HerdrCli::new("/nonexistent/herdr"),
        &target(&fake, "default"),
    );

    assert!(matches!(
        presented.connection,
        ConnectionStatus::Refused(Refusal::ExecutableMissing { .. })
    ));
}

#[test]
fn a_started_server_is_detached_and_keeps_running_after_the_client_is_done() {
    let mut fake = FakeHerdr::new();
    fake.script_server_start("work", SessionScript::new());

    open_session(&cli(&fake), &target(&fake, "work"), SessionState::Stopped);

    let server_pid = server_starts(&fake)[0].pid as libc::pid_t;
    // SAFETY: both calls only read process state and have no memory-safety preconditions.
    let (server_session, own_session, alive) = unsafe {
        (
            libc::getsid(server_pid),
            libc::getsid(0),
            libc::kill(server_pid, 0) == 0,
        )
    };
    assert!(alive, "the server must outlive the call that started it");
    assert_eq!(
        server_session, server_pid,
        "the server leads its own session"
    );
    assert_ne!(server_session, own_session);
}
