use herdr_client::{
    AgentNotification, AgentState, HerdrCli, NotificationKind, PresentedAgent, SessionName,
    SessionTarget, WorkspaceId, present_session,
};
use herdr_fake::{
    AgentScript, AgentState as ScriptedState, FakeHerdr, HerdrStatus, PaneTree, SessionScript,
    TabScript,
};

use super::*;
use crate::ai::agent_management::notifications::NotificationTarget;

fn notification(kind: NotificationKind, name: Option<&str>) -> AgentNotification {
    AgentNotification {
        kind,
        agent: PresentedAgent {
            pane: "w1:p1".into(),
            workspace: WorkspaceId::from("w1"),
            herdr_tab: "w1:t1".into(),
            kind: Some("claude".to_owned()),
            name: name.map(str::to_owned),
            state: AgentState::Blocked,
        },
    }
}

fn presented_session() -> (FakeHerdr, PresentedSession) {
    let mut fake = FakeHerdr::new();
    fake.script_status(&HerdrStatus::compatible());
    let session = fake.start_session(
        "default",
        SessionScript::new().workspace_with_tabs(
            "api",
            [TabScript::new("server", PaneTree::pane())
                .agent(0, AgentScript::new("claude", ScriptedState::Blocked))],
        ),
    );
    let presented = present_session(
        &HerdrCli::new(fake.binary_path()),
        &SessionTarget::new(&fake.config_dir(), SessionName::default_session()),
    );
    drop(session);
    (fake, presented)
}

#[test]
fn a_blocked_agent_is_announced_by_name_with_its_workspace_tab_and_pane() {
    let (_fake, session) = presented_session();

    let item = item_for(
        &notification(NotificationKind::Blocked, Some("builder")),
        &session,
    );

    assert_eq!(item.title, "builder");
    assert_eq!(
        item.message,
        "Waiting for you · claude · api › server › w1:p1"
    );
    assert_eq!(item.category, NotificationCategory::Request);
    assert_eq!(item.target, NotificationTarget::HerdrPane("w1:p1".into()));
}

#[test]
fn a_finished_unnamed_agent_is_announced_by_its_kind() {
    let (_fake, session) = presented_session();

    let item = item_for(&notification(NotificationKind::Finished, None), &session);

    assert_eq!(item.title, "claude");
    assert_eq!(item.message, "Finished · claude · api › server › w1:p1");
    assert_eq!(item.category, NotificationCategory::Complete);
}

#[test]
fn an_agent_whose_location_is_not_presented_is_announced_by_ids() {
    let item = item_for(
        &notification(NotificationKind::Blocked, None),
        &PresentedSession::connecting(),
    );

    assert_eq!(
        item.message,
        "Waiting for you · claude · w1 › w1:t1 › w1:p1"
    );
}
