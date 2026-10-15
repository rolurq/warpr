use herdr_fake::{FakeHerdr, FakeSession, SessionScript};

use super::*;
use crate::{ConnectionStatus, PresentedSession, SessionName};

fn target(fake: &FakeHerdr, name: &str) -> SessionTarget {
    SessionTarget::new(&fake.config_dir(), SessionName::new(name).unwrap())
}

fn started(fake: &FakeHerdr, name: &str, script: SessionScript) -> FakeSession {
    fake.start_session(name, script)
}

fn presented(target: &SessionTarget) -> PresentedSession {
    let snapshot = crate::fetch_snapshot(target).expect("snapshot");
    PresentedSession::from_snapshot(snapshot)
}

fn tab_labels<'a>(session: &'a PresentedSession, workspace: &str) -> Vec<&'a str> {
    session
        .workspace(&WorkspaceId::from(workspace))
        .expect("workspace is presented")
        .herdr_tabs
        .iter()
        .map(|tab| tab.label.as_str())
        .collect()
}

#[test]
fn a_created_workspace_is_in_the_session_and_focused() {
    let fake = FakeHerdr::new();
    let target = target(&fake, "main");
    let _session = started(&fake, "main", SessionScript::new().workspace("api"));

    create_workspace(&target).expect("created");

    let session = presented(&target);
    assert_eq!(session.connection, ConnectionStatus::Connected);
    assert_eq!(session.workspaces.len(), 2);
    assert_eq!(
        session.focused_workspace,
        Some(session.workspaces[1].id.clone())
    );
}

#[test]
fn a_created_tab_is_in_the_given_workspace_only_and_active() {
    let fake = FakeHerdr::new();
    let target = target(&fake, "main");
    let _session = started(
        &fake,
        "main",
        SessionScript::new().workspace("api").workspace("web"),
    );

    create_herdr_tab(&target, &WorkspaceId::from("w2")).expect("created");

    let session = presented(&target);
    assert_eq!(tab_labels(&session, "w1").len(), 1);
    assert_eq!(tab_labels(&session, "w2").len(), 2);
    let web = session.workspace(&WorkspaceId::from("w2")).unwrap();
    assert_eq!(web.active_herdr_tab, Some(web.herdr_tabs[1].id.clone()));
}

#[test]
fn a_rejected_workspace_creation_reports_herdrs_error_and_adds_nothing() {
    let fake = FakeHerdr::new();
    let target = target(&fake, "main");
    let session = started(&fake, "main", SessionScript::new().workspace("api"));
    session.reject_next_creation("server_busy", "try again later");

    let failure = create_workspace(&target).expect_err("rejected");

    assert!(matches!(
        &failure,
        ConnectionFailure::Rejected { code, message }
            if code == "server_busy" && message == "try again later"
    ));
    assert_eq!(presented(&target).workspaces.len(), 1);
}

#[test]
fn a_tab_for_a_workspace_herdr_does_not_have_is_rejected() {
    let fake = FakeHerdr::new();
    let target = target(&fake, "main");
    let _session = started(&fake, "main", SessionScript::new().workspace("api"));

    let failure = create_herdr_tab(&target, &WorkspaceId::from("w9")).expect_err("rejected");

    assert!(matches!(
        &failure,
        ConnectionFailure::Rejected { code, .. } if code == "workspace_not_found"
    ));
    assert_eq!(tab_labels(&presented(&target), "w1").len(), 1);
}

#[test]
fn a_rejected_tab_creation_reports_herdrs_error_and_adds_nothing() {
    let fake = FakeHerdr::new();
    let target = target(&fake, "main");
    let session = started(&fake, "main", SessionScript::new().workspace("api"));
    session.reject_next_creation("server_busy", "try again later");

    let failure = create_herdr_tab(&target, &WorkspaceId::from("w1")).expect_err("rejected");

    assert!(matches!(&failure, ConnectionFailure::Rejected { .. }));
    assert_eq!(tab_labels(&presented(&target), "w1").len(), 1);
}

#[test]
fn creation_in_an_unreachable_session_fails_without_a_connection() {
    let fake = FakeHerdr::new();
    let target = target(&fake, "main");
    let session = started(&fake, "main", SessionScript::new().workspace("api"));
    session.go_offline();

    assert!(matches!(
        create_workspace(&target),
        Err(ConnectionFailure::SocketUnreachable { .. })
    ));
}
