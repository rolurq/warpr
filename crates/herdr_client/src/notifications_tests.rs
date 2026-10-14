use std::sync::Arc;
use std::thread;
use std::time::Duration;

use herdr_fake::{
    AgentScript, AgentState as ScriptedState, FakeHerdr, FakeSession, HerdrStatus, PaneTree,
    SessionScript, SplitDirection, TabScript,
};
use instant::Instant;
use parking_lot::Mutex;

use super::*;
use crate::{
    AgentJump, HerdrCli, HerdrTabId, LiveAgents, PresentedSession, ReconnectPolicy, SessionName,
    SessionTarget, WorkspaceId, present_session,
};

const WAIT_LIMIT: Duration = Duration::from_secs(10);

fn fake() -> FakeHerdr {
    let mut fake = FakeHerdr::new();
    fake.script_status(&HerdrStatus::compatible());
    fake
}

fn cli(fake: &FakeHerdr) -> HerdrCli {
    HerdrCli::new(fake.binary_path())
}

fn target(fake: &FakeHerdr) -> SessionTarget {
    SessionTarget::new(
        &fake.config_dir(),
        SessionName::new("default").expect("valid name"),
    )
}

fn eventually<T>(expectation: &str, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + WAIT_LIMIT;
    loop {
        if let Some(found) = check() {
            return found;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {expectation}"
        );
        thread::sleep(Duration::from_millis(2));
    }
}

fn presented_state(state: ScriptedState) -> AgentState {
    match state {
        ScriptedState::Idle => AgentState::Idle,
        ScriptedState::Working => AgentState::Working,
        ScriptedState::Blocked => AgentState::Blocked,
        ScriptedState::Done => AgentState::Done,
        ScriptedState::Unknown => AgentState::Unknown,
    }
}

/// Workspace `api` (`w1`) has the Herdr tabs `server` (`w1:t1`, pane `w1:p1`) and `logs` (`w1:t2`,
/// panes `w1:p2` and `w1:p3`). Herdr has a named `claude` in `w1:p1` in `builder` state, and an
/// unnamed `codex` in `w1:p3` in `watcher` state.
fn fleet(builder: ScriptedState, watcher: ScriptedState) -> SessionScript {
    SessionScript::new()
        .workspace_with_tabs(
            "api",
            [
                TabScript::new("server", PaneTree::pane())
                    .agent(0, AgentScript::new("claude", builder).named("builder")),
                TabScript::new(
                    "logs",
                    PaneTree::split(
                        SplitDirection::Right,
                        0.5,
                        PaneTree::pane(),
                        PaneTree::pane(),
                    ),
                )
                .agent(1, AgentScript::new("codex", watcher)),
            ],
        )
        .workspace("web")
}

fn agent(
    pane: &str,
    herdr_tab: &str,
    kind: &str,
    name: Option<&str>,
    state: ScriptedState,
) -> PresentedAgent {
    PresentedAgent {
        pane: pane.into(),
        workspace: WorkspaceId::from("w1"),
        herdr_tab: HerdrTabId::from(herdr_tab),
        kind: Some(kind.to_owned()),
        name: name.map(str::to_owned),
        state: presented_state(state),
    }
}

fn builder(state: ScriptedState) -> PresentedAgent {
    agent("w1:p1", "w1:t1", "claude", Some("builder"), state)
}

fn raised(kind: NotificationKind, agent: PresentedAgent) -> NotificationChange {
    NotificationChange::Raised(AgentNotification { kind, agent })
}

fn retired(pane: &str, kind: NotificationKind) -> NotificationChange {
    NotificationChange::Retired {
        pane: pane.into(),
        kind,
    }
}

/// What a front-end following a session's agents would be told, with the agents themselves.
struct Following {
    _live: LiveAgents,
    tracker: Arc<Mutex<AgentNotifications>>,
    changes: Arc<Mutex<Vec<NotificationChange>>>,
    agents: Arc<Mutex<Vec<PresentedAgents>>>,
}

fn follow(fake: &FakeHerdr, policy: ReconnectPolicy) -> Following {
    let tracker = Arc::new(Mutex::new(AgentNotifications::new()));
    let changes = Arc::new(Mutex::new(Vec::new()));
    let agents = Arc::new(Mutex::new(Vec::new()));
    let live = LiveAgents::start(cli(fake), target(fake), policy, {
        let tracker = Arc::clone(&tracker);
        let changes = Arc::clone(&changes);
        let agents = Arc::clone(&agents);
        move |presented| {
            changes.lock().extend(tracker.lock().observe(&presented));
            agents.lock().push(presented);
        }
    });
    Following {
        _live: live,
        tracker,
        changes,
        agents,
    }
}

impl Following {
    fn latest(&self) -> Option<PresentedAgents> {
        self.agents.lock().last().cloned()
    }

    fn changes(&self) -> Vec<NotificationChange> {
        self.changes.lock().clone()
    }

    fn wait_for_state(&self, pane: &str, state: ScriptedState) {
        eventually(&format!("{pane} to be {state:?}"), || {
            let latest = self.latest()?;
            (latest.connection == ConnectionStatus::Connected
                && latest
                    .agent(&pane.into())
                    .is_some_and(|agent| agent.state == presented_state(state)))
            .then_some(())
        });
    }

    fn wait_for_connected(&self) {
        eventually("a connection", || {
            (self.latest()?.connection == ConnectionStatus::Connected).then_some(())
        });
    }

    fn view(&self, panes: &[&str]) {
        let changes = self
            .tracker
            .lock()
            .set_viewed(panes.iter().map(|pane| PaneId::from(*pane)));
        self.changes.lock().extend(changes);
    }
}

/// Has Herdr report `builder` as the state of `w1:p1` and waits for it to be presented.
fn change_builder(session: &FakeSession, following: &Following, state: ScriptedState) {
    session.set_script(fleet(state, ScriptedState::Idle));
    session.emit_agent_status_changed("w1:p1", "claude", state);
    following.wait_for_state("w1:p1", state);
}

fn wait_for_status_subscriptions(session: &FakeSession) {
    let panes = ["w1:p1", "w1:p2", "w1:p3", "w2:p1"];
    eventually("status subscriptions", || {
        (session.agent_status_subscriptions() == panes).then_some(())
    });
}

fn authoritative(fake: &FakeHerdr) -> PresentedSession {
    present_session(&cli(fake), &target(fake))
}

#[test]
fn an_agent_becoming_blocked_is_announced_with_its_identity_and_location() {
    let fake = fake();
    let session = fake.start_session(
        "default",
        fleet(ScriptedState::Working, ScriptedState::Idle),
    );
    let following = follow(&fake, ReconnectPolicy::default());
    following.wait_for_state("w1:p1", ScriptedState::Working);
    wait_for_status_subscriptions(&session);

    change_builder(&session, &following, ScriptedState::Blocked);

    let changes = following.changes();
    assert_eq!(
        changes,
        [raised(
            NotificationKind::Blocked,
            builder(ScriptedState::Blocked)
        )]
    );
    let NotificationChange::Raised(notification) = &changes[0] else {
        unreachable!("asserted above")
    };
    let session = authoritative(&fake);
    let location = notification
        .agent
        .location(&session)
        .expect("the agent's location is presented");
    assert_eq!(location.workspace.label, "api");
    assert_eq!(location.herdr_tab.label, "server");
    assert_eq!(
        notification.agent.jump_target(&session),
        Some(AgentJump {
            workspace: "w1".into(),
            herdr_tab: "w1:t1".into(),
            pane: "w1:p1".into(),
        })
    );
}

#[test]
fn an_agent_finishing_work_is_announced() {
    let fake = fake();
    let session = fake.start_session(
        "default",
        fleet(ScriptedState::Working, ScriptedState::Idle),
    );
    let following = follow(&fake, ReconnectPolicy::default());
    following.wait_for_state("w1:p1", ScriptedState::Working);
    wait_for_status_subscriptions(&session);

    change_builder(&session, &following, ScriptedState::Done);

    assert_eq!(
        following.changes(),
        [raised(
            NotificationKind::Finished,
            builder(ScriptedState::Done)
        )]
    );
}

#[test]
fn becoming_idle_is_not_announced() {
    let fake = fake();
    let session = fake.start_session(
        "default",
        fleet(ScriptedState::Working, ScriptedState::Idle),
    );
    let following = follow(&fake, ReconnectPolicy::default());
    following.wait_for_state("w1:p1", ScriptedState::Working);
    wait_for_status_subscriptions(&session);

    change_builder(&session, &following, ScriptedState::Idle);

    assert_eq!(following.changes(), []);
}

#[test]
fn states_present_when_first_connecting_are_not_announced() {
    let fake = fake();
    let session = fake.start_session(
        "default",
        fleet(ScriptedState::Blocked, ScriptedState::Done),
    );
    let following = follow(&fake, ReconnectPolicy::default());
    following.wait_for_state("w1:p1", ScriptedState::Blocked);
    wait_for_status_subscriptions(&session);

    assert_eq!(following.changes(), []);

    // The same agent leaving and re-entering `blocked` does announce.
    change_builder(&session, &following, ScriptedState::Working);
    change_builder(&session, &following, ScriptedState::Blocked);
    assert_eq!(
        following.changes(),
        [raised(
            NotificationKind::Blocked,
            builder(ScriptedState::Blocked)
        )]
    );
}

#[test]
fn an_unchanged_state_after_reconnecting_is_not_announced() {
    let fake = fake();
    let session = fake.start_session(
        "default",
        fleet(ScriptedState::Working, ScriptedState::Idle),
    );
    let following = follow(&fake, quick_retries());
    following.wait_for_state("w1:p1", ScriptedState::Working);
    change_builder(&session, &following, ScriptedState::Blocked);
    assert_eq!(following.changes().len(), 1);

    session.drop_connections();
    eventually("the drop to show", || {
        following
            .agents
            .lock()
            .iter()
            .any(|presented| matches!(presented.connection, ConnectionStatus::Reconnecting(_)))
            .then_some(())
    });
    following.wait_for_connected();
    wait_for_status_subscriptions(&session);

    assert_eq!(
        following.changes().len(),
        1,
        "reconnecting announced again: {:?}",
        following.changes()
    );
}

#[test]
fn a_state_that_changed_during_an_outage_is_announced_once_reconnected() {
    let fake = fake();
    let session = fake.start_session(
        "default",
        fleet(ScriptedState::Working, ScriptedState::Idle),
    );
    let following = follow(&fake, quick_retries());
    following.wait_for_state("w1:p1", ScriptedState::Working);

    session.go_offline();
    eventually("the outage to show", || {
        following
            .latest()
            .filter(|presented| matches!(presented.connection, ConnectionStatus::Reconnecting(_)))
            .map(|_| ())
    });
    session.set_script(fleet(ScriptedState::Blocked, ScriptedState::Idle));
    session.go_online();
    following.wait_for_state("w1:p1", ScriptedState::Blocked);

    assert_eq!(
        following.changes(),
        [raised(
            NotificationKind::Blocked,
            builder(ScriptedState::Blocked)
        )]
    );
}

#[test]
fn reading_the_same_state_again_never_announces_again() {
    let fake = fake();
    let session = fake.start_session(
        "default",
        fleet(ScriptedState::Working, ScriptedState::Idle),
    );
    let following = follow(&fake, ReconnectPolicy::default());
    following.wait_for_state("w1:p1", ScriptedState::Working);
    wait_for_status_subscriptions(&session);
    change_builder(&session, &following, ScriptedState::Blocked);

    // Another agent's change forces a fresh read that includes the still-blocked agent.
    session.set_script(fleet(ScriptedState::Blocked, ScriptedState::Working));
    session.emit_agent_status_changed("w1:p3", "codex", ScriptedState::Working);
    following.wait_for_state("w1:p3", ScriptedState::Working);

    assert_eq!(following.changes().len(), 1);
}

#[test]
fn leaving_a_state_retires_its_notification_and_entering_it_again_announces_again() {
    let fake = fake();
    let session = fake.start_session(
        "default",
        fleet(ScriptedState::Working, ScriptedState::Idle),
    );
    let following = follow(&fake, ReconnectPolicy::default());
    following.wait_for_state("w1:p1", ScriptedState::Working);
    wait_for_status_subscriptions(&session);

    change_builder(&session, &following, ScriptedState::Blocked);
    change_builder(&session, &following, ScriptedState::Working);
    change_builder(&session, &following, ScriptedState::Blocked);
    change_builder(&session, &following, ScriptedState::Done);

    let blocked = || raised(NotificationKind::Blocked, builder(ScriptedState::Blocked));
    assert_eq!(
        following.changes(),
        [
            blocked(),
            retired("w1:p1", NotificationKind::Blocked),
            blocked(),
            retired("w1:p1", NotificationKind::Blocked),
        ]
    );
}

#[test]
fn an_agent_that_disappears_retires_its_notification() {
    let fake = fake();
    let session = fake.start_session(
        "default",
        fleet(ScriptedState::Working, ScriptedState::Idle),
    );
    let following = follow(&fake, ReconnectPolicy::default());
    following.wait_for_state("w1:p1", ScriptedState::Working);
    wait_for_status_subscriptions(&session);
    change_builder(&session, &following, ScriptedState::Blocked);

    session.set_script(SessionScript::new().workspace("api"));
    session.emit_event("pane_closed", serde_json::json!({ "pane_id": "w1:p1" }));
    eventually("the agent to go", || {
        following
            .latest()
            .filter(|presented| presented.agents.is_empty())
            .map(|_| ())
    });

    assert_eq!(
        following.changes().last(),
        Some(&retired("w1:p1", NotificationKind::Blocked))
    );
}

#[test]
fn viewing_an_agents_pane_retires_its_finished_notification() {
    let fake = fake();
    let session = fake.start_session(
        "default",
        fleet(ScriptedState::Working, ScriptedState::Idle),
    );
    let following = follow(&fake, ReconnectPolicy::default());
    following.wait_for_state("w1:p1", ScriptedState::Working);
    wait_for_status_subscriptions(&session);
    change_builder(&session, &following, ScriptedState::Done);

    following.view(&["w1:p2"]);
    assert_eq!(following.changes().len(), 1);
    following.view(&["w1:p1"]);

    assert_eq!(
        following.changes().last(),
        Some(&retired("w1:p1", NotificationKind::Finished))
    );
}

#[test]
fn finishing_while_its_pane_is_in_view_announces_nothing_and_later_work_does() {
    let fake = fake();
    let session = fake.start_session(
        "default",
        fleet(ScriptedState::Working, ScriptedState::Idle),
    );
    let following = follow(&fake, ReconnectPolicy::default());
    following.wait_for_state("w1:p1", ScriptedState::Working);
    wait_for_status_subscriptions(&session);
    following.view(&["w1:p1"]);

    change_builder(&session, &following, ScriptedState::Done);
    assert_eq!(following.changes(), []);

    following.view(&[]);
    change_builder(&session, &following, ScriptedState::Working);
    change_builder(&session, &following, ScriptedState::Done);
    assert_eq!(
        following.changes(),
        [raised(
            NotificationKind::Finished,
            builder(ScriptedState::Done)
        )]
    );
}

#[test]
fn viewing_a_blocked_agents_pane_keeps_its_notification_until_it_is_unblocked() {
    let fake = fake();
    let session = fake.start_session(
        "default",
        fleet(ScriptedState::Working, ScriptedState::Idle),
    );
    let following = follow(&fake, ReconnectPolicy::default());
    following.wait_for_state("w1:p1", ScriptedState::Working);
    wait_for_status_subscriptions(&session);
    change_builder(&session, &following, ScriptedState::Blocked);

    following.view(&["w1:p1"]);

    assert_eq!(following.changes().len(), 1);
}

fn quick_retries() -> ReconnectPolicy {
    ReconnectPolicy {
        initial_backoff: Duration::from_millis(5),
        max_backoff: Duration::from_millis(20),
        disconnected_after: 1000,
    }
}
