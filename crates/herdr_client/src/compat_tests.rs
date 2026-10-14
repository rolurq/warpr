use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use herdr_fake::{FakeHerdr, FakeSession, HerdrStatus, SessionScript};

use super::*;
use crate::{ConnectionFailure, ConnectionStatus, PresentedSession, SessionName, present_session};

fn fake_with(status: HerdrStatus) -> FakeHerdr {
    let mut fake = FakeHerdr::new();
    fake.script_status(&status);
    fake
}

fn target(fake: &FakeHerdr) -> SessionTarget {
    SessionTarget::new(&fake.config_dir(), SessionName::default_session())
}

/// A session that would present workspaces if warpr connected to it.
fn serving_session(fake: &FakeHerdr) -> FakeSession {
    fake.start_session("default", SessionScript::new().workspace("api"))
}

fn present_with(cli: &HerdrCli, fake: &FakeHerdr) -> PresentedSession {
    present_session(cli, &target(fake))
}

fn refusal_of(presented: &PresentedSession) -> &Refusal {
    match &presented.connection {
        ConnectionStatus::Refused(refusal) => refusal,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn assert_nothing_presented(presented: &PresentedSession, session: &FakeSession) {
    assert!(presented.workspaces.is_empty());
    assert_eq!(presented.focused_workspace, None);
    assert_eq!(session.requests_served(), 0);
}

#[test]
fn a_compatible_herdr_connects_and_presents_the_session() {
    let fake = fake_with(HerdrStatus::compatible());
    let _session = serving_session(&fake);

    let presented = present_with(&HerdrCli::new(fake.binary_path()), &fake);

    assert_eq!(presented.connection, ConnectionStatus::Connected);
    assert_eq!(presented.workspaces.len(), 1);
}

#[test]
fn a_missing_executable_is_refused_with_what_to_install() {
    let fake = FakeHerdr::new();
    let session = serving_session(&fake);
    let missing = fake.config_dir().join("not-installed").join("herdr");

    let presented = present_with(&HerdrCli::new(&missing), &fake);

    assert_eq!(
        refusal_of(&presented),
        &Refusal::ExecutableMissing {
            binary: missing.clone()
        }
    );
    let message = refusal_of(&presented).to_string();
    assert!(message.contains("Install Herdr"), "{message}");
    assert!(
        message.contains(&missing.display().to_string()),
        "{message}"
    );
    assert!(message.contains(HERDR_BINARY_ENV), "{message}");
    assert_nothing_presented(&presented, &session);
}

#[test]
fn an_executable_that_cannot_be_run_is_refused_with_why() {
    let fake = fake_with(HerdrStatus::compatible());
    let session = serving_session(&fake);
    fs::set_permissions(fake.binary_path(), fs::Permissions::from_mode(0o644)).unwrap();

    let presented = present_with(&HerdrCli::new(fake.binary_path()), &fake);

    let Refusal::ExecutableNotRunnable { binary, reason } = refusal_of(&presented) else {
        panic!(
            "expected ExecutableNotRunnable, got {:?}",
            presented.connection
        );
    };
    assert_eq!(binary, fake.binary_path());
    assert!(
        reason.to_lowercase().contains("permission denied"),
        "{reason}"
    );
    let message = refusal_of(&presented).to_string();
    assert!(message.contains("cannot be run"), "{message}");
    assert!(message.contains("Reinstall Herdr"), "{message}");
    assert_nothing_presented(&presented, &session);
}

#[test]
fn an_incompatible_endpoint_is_refused_naming_what_warpr_needs_and_what_herdr_reports() {
    let fake = fake_with(
        HerdrStatus::compatible()
            .server_version("0.9.9")
            .server_endpoint_generation(Some(2)),
    );
    let session = serving_session(&fake);

    let presented = present_with(&HerdrCli::new(fake.binary_path()), &fake);

    assert_eq!(
        refusal_of(&presented),
        &Refusal::IncompatibleEndpoint {
            executable_version: "0.9.3".to_owned(),
            needed: 1,
            server_version: Some("0.9.9".to_owned()),
            reported: Some(2),
        }
    );
    let message = refusal_of(&presented).to_string();
    assert!(message.contains("endpoint is incompatible"), "{message}");
    assert!(message.contains("generation 2"), "{message}");
    assert!(message.contains("needs generation 1"), "{message}");
    assert!(message.contains("0.9.9"), "{message}");
    assert_nothing_presented(&presented, &session);
}

#[test]
fn a_server_that_reports_no_endpoint_generation_is_refused_as_unverifiable() {
    let fake = fake_with(HerdrStatus::compatible().server_endpoint_generation(None));
    let session = serving_session(&fake);

    let presented = present_with(&HerdrCli::new(fake.binary_path()), &fake);

    assert!(matches!(
        refusal_of(&presented),
        Refusal::IncompatibleEndpoint {
            reported: None,
            needed: 1,
            ..
        }
    ));
    let message = refusal_of(&presented).to_string();
    assert!(message.contains("generation unknown"), "{message}");
    assert_nothing_presented(&presented, &session);
}

#[test]
fn an_incompatible_protocol_is_refused_naming_what_warpr_needs_and_what_herdr_reports() {
    let fake = fake_with(
        HerdrStatus::compatible()
            .server_version("0.9.1")
            .server_protocol(21),
    );
    let session = serving_session(&fake);

    let presented = present_with(&HerdrCli::new(fake.binary_path()), &fake);

    assert_eq!(
        refusal_of(&presented),
        &Refusal::IncompatibleProtocol {
            executable_version: "0.9.3".to_owned(),
            needed: 22,
            server_version: Some("0.9.1".to_owned()),
            reported: Some(21),
        }
    );
    let message = refusal_of(&presented).to_string();
    assert!(
        message.contains("private protocol is incompatible"),
        "{message}"
    );
    assert!(message.contains("speaks protocol 21"), "{message}");
    assert!(message.contains("needs protocol 22"), "{message}");
    assert_nothing_presented(&presented, &session);
}

#[test]
fn an_incompatible_endpoint_is_reported_ahead_of_an_incompatible_protocol() {
    let fake = fake_with(
        HerdrStatus::compatible()
            .server_protocol(21)
            .server_endpoint_generation(Some(0)),
    );
    let _session = serving_session(&fake);

    let presented = present_with(&HerdrCli::new(fake.binary_path()), &fake);

    assert!(matches!(
        refusal_of(&presented),
        Refusal::IncompatibleEndpoint { .. }
    ));
}

#[test]
fn a_session_whose_server_is_not_running_is_not_refused() {
    let fake = fake_with(HerdrStatus::server_not_running());

    let presented = present_with(&HerdrCli::new(fake.binary_path()), &fake);

    assert!(matches!(
        presented.connection,
        ConnectionStatus::Failed(ConnectionFailure::SocketUnreachable { .. })
    ));
}

#[test]
fn a_failing_status_command_is_refused_with_its_exit_code_and_complaint() {
    let mut fake = FakeHerdr::new();
    fake.script_command(&["status", "server", "--json"], "", 1);
    let session = serving_session(&fake);

    let presented = present_with(&HerdrCli::new(fake.binary_path()), &fake);

    let Refusal::StatusUnavailable { detail } = refusal_of(&presented) else {
        panic!("expected StatusUnavailable, got {:?}", presented.connection);
    };
    assert!(detail.contains("herdr status server"), "{detail}");
    assert!(detail.contains("exit status: 1"), "{detail}");
    assert_nothing_presented(&presented, &session);
}

#[test]
fn status_output_that_is_not_herdrs_is_refused() {
    let mut fake = FakeHerdr::new();
    fake.script_command(
        &["status", "server", "--json"],
        "{\"hello\":\"world\"}\n",
        0,
    );
    let session = serving_session(&fake);

    let presented = present_with(&HerdrCli::new(fake.binary_path()), &fake);

    let Refusal::StatusUnavailable { detail } = refusal_of(&presented) else {
        panic!("expected StatusUnavailable, got {:?}", presented.connection);
    };
    assert!(detail.contains("not understood"), "{detail}");
    assert_nothing_presented(&presented, &session);
}

#[test]
fn a_status_command_that_never_answers_is_refused_after_the_timeout() {
    let mut fake = FakeHerdr::new();
    fake.script_hang(&["status", "server", "--json"]);

    let result = check(
        &HerdrCli::new(fake.binary_path()),
        &target(&fake),
        Duration::from_millis(200),
    );

    let Err(Refusal::StatusUnavailable { detail }) = result else {
        panic!("expected StatusUnavailable, got {result:?}");
    };
    assert!(detail.contains("did not finish within"), "{detail}");
}
