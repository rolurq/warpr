use std::sync::Arc;
use std::thread;
use std::time::Duration;

use herdr_fake::{
    AgentScript, AgentState, FakeHerdr, FakeSession, HerdrStatus, PaneTree, SessionScript,
    SplitDirection, TabScript,
};
use instant::Instant;
use parking_lot::Mutex;

use super::*;
use crate::{
    ConnectionStatus, HerdrTabId, PresentedAgent, PresentedSession, Refusal, SessionName,
    WorkspaceId, present_session,
};

const WAIT_LIMIT: Duration = Duration::from_secs(10);

fn quick_retries(disconnected_after: u32) -> ReconnectPolicy {
    ReconnectPolicy {
        initial_backoff: Duration::from_millis(5),
        max_backoff: Duration::from_millis(20),
        disconnected_after,
    }
}

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

fn two_panes() -> PaneTree {
    PaneTree::split(
        SplitDirection::Right,
        0.5,
        PaneTree::pane(),
        PaneTree::pane(),
    )
}

/// Workspace `api` (`w1`) has the Herdr tabs `server` (`w1:t1`, pane `w1:p1`) and `logs` (`w1:t2`,
/// panes `w1:p2` and `w1:p3`); workspace `web` (`w2`) has the one pane `w2:p1`. Herdr has an agent
/// in `w1:p1`, named, and an unnamed one in `w1:p3`.
fn fleet(builder: AgentState) -> SessionScript {
    SessionScript::new()
        .workspace_with_tabs(
            "api",
            [
                TabScript::new("server", PaneTree::pane())
                    .agent(0, AgentScript::new("claude", builder).named("builder")),
                TabScript::new("logs", two_panes())
                    .agent(1, AgentScript::new("codex", AgentState::Idle)),
            ],
        )
        .workspace("web")
}

fn agent(
    pane: &str,
    workspace: &str,
    herdr_tab: &str,
    kind: &str,
    name: Option<&str>,
    state: AgentState,
) -> PresentedAgent {
    PresentedAgent {
        pane: pane.into(),
        workspace: WorkspaceId::from(workspace),
        herdr_tab: HerdrTabId::from(herdr_tab),
        kind: Some(kind.to_owned()),
        name: name.map(str::to_owned),
        state: presented_state(state),
    }
}

/// What the client presents for an agent the fake scripts in `state`.
fn presented_state(state: AgentState) -> crate::AgentState {
    match state {
        AgentState::Idle => crate::AgentState::Idle,
        AgentState::Working => crate::AgentState::Working,
        AgentState::Blocked => crate::AgentState::Blocked,
        AgentState::Done => crate::AgentState::Done,
        AgentState::Unknown => crate::AgentState::Unknown,
    }
}

/// A [`LiveAgents`] together with everything it presented.
struct Observed {
    live: LiveAgents,
    updates: Arc<Mutex<Vec<PresentedAgents>>>,
}

fn observe(fake: &FakeHerdr, policy: ReconnectPolicy) -> Observed {
    observe_with(cli(fake), fake, policy)
}

fn observe_with(cli: HerdrCli, fake: &FakeHerdr, policy: ReconnectPolicy) -> Observed {
    let updates = Arc::new(Mutex::new(Vec::new()));
    let live = LiveAgents::start(cli, target(fake), policy, {
        let updates = Arc::clone(&updates);
        move |presented| updates.lock().push(presented)
    });
    Observed { live, updates }
}

impl Observed {
    fn latest(&self) -> PresentedAgents {
        self.updates
            .lock()
            .last()
            .cloned()
            .unwrap_or_else(PresentedAgents::connecting)
    }

    fn history(&self) -> Vec<PresentedAgents> {
        self.updates.lock().clone()
    }

    fn wait_for(
        &self,
        expectation: &str,
        mut matches: impl FnMut(&PresentedAgents) -> bool,
    ) -> PresentedAgents {
        eventually(expectation, || {
            let latest = self.latest();
            matches(&latest).then_some(latest)
        })
    }

    fn wait_for_connected(
        &self,
        expectation: &str,
        mut matches: impl FnMut(&PresentedAgents) -> bool,
    ) -> PresentedAgents {
        self.wait_for(expectation, |presented| {
            presented.connection == ConnectionStatus::Connected && matches(presented)
        })
    }

    fn wait_for_state(&self, pane: &str, state: AgentState) -> PresentedAgents {
        self.wait_for_connected(&format!("{pane} to be {state:?}"), |presented| {
            presented
                .agent(&pane.into())
                .is_some_and(|agent| agent.state == presented_state(state))
        })
    }
}

fn is_refused(presented: &PresentedAgents) -> bool {
    matches!(presented.connection, ConnectionStatus::Refused(_))
}

fn is_reconnecting(presented: &PresentedAgents) -> bool {
    matches!(presented.connection, ConnectionStatus::Reconnecting(_))
}

fn panes_of(presented: &PresentedAgents) -> Vec<&str> {
    presented
        .agents
        .iter()
        .map(|agent| agent.pane.as_str())
        .collect()
}

/// Waits until the open subscriptions follow exactly `panes`.
fn wait_for_status_subscriptions(session: &FakeSession, panes: &[&str]) {
    eventually(&format!("status subscriptions for {panes:?}"), || {
        (session.agent_status_subscriptions() == panes).then_some(())
    });
}

fn authoritative(fake: &FakeHerdr) -> PresentedSession {
    present_session(&cli(fake), &target(fake))
}

#[test]
fn every_agent_is_listed_with_name_kind_state_and_location() {
    let fake = fake();
    let _session = fake.start_session("default", fleet(AgentState::Blocked));

    let observed = observe(&fake, ReconnectPolicy::default());

    let presented =
        observed.wait_for_connected("the agents", |presented| !presented.agents.is_empty());
    assert_eq!(
        presented.agents,
        [
            agent(
                "w1:p1",
                "w1",
                "w1:t1",
                "claude",
                Some("builder"),
                AgentState::Blocked
            ),
            agent("w1:p3", "w1", "w1:t2", "codex", None, AgentState::Idle),
        ]
    );
    let session = authoritative(&fake);
    let location = presented.agents[1]
        .location(&session)
        .expect("the agent's workspace and Herdr tab are presented");
    assert_eq!(location.workspace.label, "api");
    assert_eq!(location.herdr_tab.label, "logs");
}

#[test]
fn a_session_without_agents_presents_none() {
    let fake = fake();
    let _session = fake.start_session("default", SessionScript::new().workspace("api"));

    let observed = observe(&fake, ReconnectPolicy::default());

    let presented = observed.wait_for_connected("a connection", |_| true);
    assert!(presented.agents.is_empty());
}

#[test]
fn the_agents_are_connecting_until_the_first_read_arrives() {
    let fake = fake();
    let session = fake.start_session("default", fleet(AgentState::Idle));
    let pause = session.pause_snapshots();
    let observed = observe(&fake, ReconnectPolicy::default());
    pause.wait_until_requested();

    assert_eq!(observed.latest(), PresentedAgents::connecting());
    drop(pause);
    observed.wait_for_connected("the agents", |presented| !presented.agents.is_empty());
}

#[test]
fn every_pane_is_subscribed_to_for_status_changes_not_only_those_with_agents() {
    let fake = fake();
    let session = fake.start_session("default", fleet(AgentState::Idle));
    let _observed = observe(&fake, ReconnectPolicy::default());

    wait_for_status_subscriptions(&session, &["w1:p1", "w1:p2", "w1:p3", "w2:p1"]);
}

#[test]
fn a_state_change_appears_when_herdr_announces_it() {
    let fake = fake();
    let session = fake.start_session("default", fleet(AgentState::Working));
    let observed = observe(&fake, ReconnectPolicy::default());
    observed.wait_for_state("w1:p1", AgentState::Working);
    wait_for_status_subscriptions(&session, &["w1:p1", "w1:p2", "w1:p3", "w2:p1"]);

    session.set_script(fleet(AgentState::Blocked));
    let delivered = session.emit_agent_status_changed("w1:p1", "claude", AgentState::Blocked);
    assert_eq!(delivered, 1);

    let presented = observed.wait_for_state("w1:p1", AgentState::Blocked);
    assert_eq!(
        presented.agent(&"w1:p3".into()).map(|a| a.state),
        Some(presented_state(AgentState::Idle))
    );
}

#[test]
fn successive_state_changes_all_show() {
    let fake = fake();
    let session = fake.start_session("default", fleet(AgentState::Working));
    let observed = observe(&fake, ReconnectPolicy::default());
    wait_for_status_subscriptions(&session, &["w1:p1", "w1:p2", "w1:p3", "w2:p1"]);

    for state in [AgentState::Blocked, AgentState::Working, AgentState::Done] {
        session.set_script(fleet(state));
        session.emit_agent_status_changed("w1:p1", "claude", state);
        observed.wait_for_state("w1:p1", state);
    }
}

#[test]
fn an_events_payload_is_never_taken_as_the_new_state() {
    let fake = fake();
    let session = fake.start_session("default", fleet(AgentState::Working));
    let observed = observe(&fake, ReconnectPolicy::default());
    wait_for_status_subscriptions(&session, &["w1:p1", "w1:p2", "w1:p3", "w2:p1"]);

    // Herdr's answer is `done`; the event, as one overtaken by a later change would, says `idle`.
    session.set_script(fleet(AgentState::Done));
    session.emit_agent_status_changed("w1:p1", "claude", AgentState::Idle);

    observed.wait_for_state("w1:p1", AgentState::Done);
}

#[test]
fn a_read_overtaken_by_a_state_change_is_read_again() {
    let fake = fake();
    let session = fake.start_session("default", fleet(AgentState::Working));
    let observed = observe(&fake, ReconnectPolicy::default());
    observed.wait_for_state("w1:p1", AgentState::Working);
    wait_for_status_subscriptions(&session, &["w1:p1", "w1:p2", "w1:p3", "w2:p1"]);

    let pause = session.pause_snapshots();
    session.set_script(fleet(AgentState::Done));
    session.emit_agent_status_changed("w1:p1", "claude", AgentState::Done);
    // The read this change prompts is held, already describing `done`; Herdr moves on.
    pause.wait_until_requested();
    session.set_script(fleet(AgentState::Blocked));
    session.emit_agent_status_changed("w1:p1", "claude", AgentState::Blocked);
    drop(pause);

    observed.wait_for_state("w1:p1", AgentState::Blocked);
}

#[test]
fn an_agent_that_appears_or_disappears_is_presented_without_a_status_event() {
    let fake = fake();
    let session = fake.start_session("default", fleet(AgentState::Working));
    let observed = observe(&fake, ReconnectPolicy::default());
    observed.wait_for_connected("the agents", |presented| presented.agents.len() == 2);

    session.set_script(fleet(AgentState::Working).agent_in(
        "web",
        "1",
        0,
        AgentScript::new("pi", AgentState::Idle),
    ));
    session.emit_event("pane_agent_detected", serde_json::json!({}));
    let presented =
        observed.wait_for_connected("the new agent", |presented| presented.agents.len() == 3);
    assert_eq!(
        presented.agent(&"w2:p1".into()),
        Some(&agent("w2:p1", "w2", "w2:t1", "pi", None, AgentState::Idle))
    );

    session.set_script(fleet(AgentState::Working));
    session.emit_event("pane_agent_detected", serde_json::json!({}));
    observed.wait_for_connected("the agent to be gone", |presented| {
        presented.agent(&"w2:p1".into()).is_none()
    });
}

#[test]
fn an_agent_renamed_elsewhere_shows_its_new_name() {
    let fake = fake();
    let session = fake.start_session("default", fleet(AgentState::Working));
    let observed = observe(&fake, ReconnectPolicy::default());
    observed.wait_for_connected("the agents", |presented| presented.agents.len() == 2);

    session.set_script(
        SessionScript::new()
            .workspace_with_tabs(
                "api",
                [
                    TabScript::new("server", PaneTree::pane()).agent(
                        0,
                        AgentScript::new("claude", AgentState::Working).named("reviewer"),
                    ),
                    TabScript::new("logs", two_panes())
                        .agent(1, AgentScript::new("codex", AgentState::Idle)),
                ],
            )
            .workspace("web"),
    );
    session.emit_event("pane_updated", serde_json::json!({}));

    observed.wait_for_connected("the new name", |presented| {
        presented
            .agent(&"w1:p1".into())
            .is_some_and(|agent| agent.name.as_deref() == Some("reviewer"))
    });
}

fn web_with(tree: PaneTree, agent_pane: usize) -> SessionScript {
    SessionScript::new()
        .workspace_with_tabs(
            "api",
            [
                TabScript::new("server", PaneTree::pane())
                    .agent(0, AgentScript::new("claude", AgentState::Idle)),
                TabScript::new("logs", two_panes()),
            ],
        )
        .workspace_with_tabs(
            "web",
            [TabScript::new("1", tree)
                .agent(agent_pane, AgentScript::new("pi", AgentState::Working))],
        )
}

#[test]
fn a_pane_that_appears_while_connected_has_its_state_changes_followed() {
    let fake = fake();
    let session = fake.start_session("default", web_with(PaneTree::pane(), 0));
    let observed = observe(&fake, ReconnectPolicy::default());
    wait_for_status_subscriptions(&session, &["w1:p1", "w1:p2", "w1:p3", "w2:p1"]);

    session.set_script(web_with(two_panes(), 1));
    session.emit_event("pane_created", serde_json::json!({}));
    wait_for_status_subscriptions(&session, &["w1:p1", "w1:p2", "w1:p3", "w2:p1", "w2:p2"]);
    observed.wait_for_state("w2:p2", AgentState::Working);

    let blocked = SessionScript::new()
        .workspace_with_tabs(
            "api",
            [
                TabScript::new("server", PaneTree::pane())
                    .agent(0, AgentScript::new("claude", AgentState::Idle)),
                TabScript::new("logs", two_panes()),
            ],
        )
        .workspace_with_tabs(
            "web",
            [TabScript::new("1", two_panes())
                .agent(1, AgentScript::new("pi", AgentState::Blocked))],
        );
    session.set_script(blocked);
    let delivered = session.emit_agent_status_changed("w2:p2", "pi", AgentState::Blocked);
    assert_eq!(delivered, 1);

    observed.wait_for_state("w2:p2", AgentState::Blocked);
}

#[test]
fn a_pane_that_closes_while_connected_is_dropped_with_its_agent() {
    let fake = fake();
    let session = fake.start_session("default", web_with(two_panes(), 1));
    let observed = observe(&fake, ReconnectPolicy::default());
    observed.wait_for_state("w2:p2", AgentState::Working);
    wait_for_status_subscriptions(&session, &["w1:p1", "w1:p2", "w1:p3", "w2:p1", "w2:p2"]);

    session.set_script(web_with(PaneTree::pane(), 0));
    session.emit_event("pane_closed", serde_json::json!({}));

    wait_for_status_subscriptions(&session, &["w1:p1", "w1:p2", "w1:p3", "w2:p1"]);
    let presented = observed.wait_for_connected("the closed pane's agent to go", |presented| {
        presented.agent(&"w2:p2".into()).is_none()
    });
    assert_eq!(panes_of(&presented), ["w1:p1", "w2:p1"]);
}

#[test]
fn lost_events_are_recovered_by_reading_again_and_subscribing_afresh() {
    let fake = fake();
    let session = fake.start_session("default", fleet(AgentState::Working));
    let observed = observe(&fake, ReconnectPolicy::default());
    observed.wait_for_state("w1:p1", AgentState::Working);
    wait_for_status_subscriptions(&session, &["w1:p1", "w1:p2", "w1:p3", "w2:p1"]);

    session.set_script(fleet(AgentState::Blocked));
    session.lose_events();

    observed.wait_for_state("w1:p1", AgentState::Blocked);
    wait_for_status_subscriptions(&session, &["w1:p1", "w1:p2", "w1:p3", "w2:p1"]);
    session.set_script(fleet(AgentState::Done));
    session.emit_agent_status_changed("w1:p1", "claude", AgentState::Done);
    observed.wait_for_state("w1:p1", AgentState::Done);
}

#[test]
fn a_session_that_goes_away_keeps_its_agents_while_reconnecting_and_is_reconciled_after() {
    let fake = fake();
    let session = fake.start_session("default", fleet(AgentState::Working));
    let observed = observe(&fake, quick_retries(1000));
    observed.wait_for_state("w1:p1", AgentState::Working);

    session.go_offline();

    let presented = observed.wait_for("reconnecting", is_reconnecting);
    assert_eq!(panes_of(&presented), ["w1:p1", "w1:p3"]);

    session.set_script(fleet(AgentState::Blocked));
    session.go_online();

    observed.wait_for_state("w1:p1", AgentState::Blocked);
    wait_for_status_subscriptions(&session, &["w1:p1", "w1:p2", "w1:p3", "w2:p1"]);
    session.set_script(fleet(AgentState::Done));
    session.emit_agent_status_changed("w1:p1", "claude", AgentState::Done);
    observed.wait_for_state("w1:p1", AgentState::Done);
}

#[test]
fn a_dropped_connection_is_retried_and_the_state_reconciled() {
    let fake = fake();
    let session = fake.start_session("default", fleet(AgentState::Working));
    let observed = observe(&fake, quick_retries(10));
    observed.wait_for_state("w1:p1", AgentState::Working);

    session.set_script(fleet(AgentState::Blocked));
    session.drop_connections();

    observed.wait_for_state("w1:p1", AgentState::Blocked);
    let history = observed.history();
    assert!(
        history.iter().any(is_reconnecting),
        "connection health never showed the drop: {history:?}"
    );
}

#[test]
fn asking_to_reconnect_retries_now() {
    let fake = fake();
    let session = fake.start_session("default", fleet(AgentState::Working));
    let observed = observe(
        &fake,
        ReconnectPolicy {
            initial_backoff: Duration::from_secs(3600),
            max_backoff: Duration::from_secs(3600),
            disconnected_after: 10,
        },
    );
    observed.wait_for_state("w1:p1", AgentState::Working);

    session.go_offline();
    observed.wait_for("reconnecting", is_reconnecting);
    session.set_script(fleet(AgentState::Blocked));
    session.go_online();
    observed.live.reconnect();

    observed.wait_for_state("w1:p1", AgentState::Blocked);
}

#[test]
fn an_incompatible_herdr_is_refused_and_no_agents_are_presented() {
    let mut fake = fake();
    fake.script_status(&HerdrStatus::compatible().server_protocol(21));
    let session = fake.start_session("default", fleet(AgentState::Working));

    let observed = observe(&fake, quick_retries(10));

    let presented = observed.wait_for("a refusal", is_refused);
    assert!(presented.agents.is_empty());
    assert_eq!(session.subscriber_count(), 0);
}

#[test]
fn a_missing_herdr_executable_is_refused() {
    let fake = fake();
    let _session = fake.start_session("default", fleet(AgentState::Working));
    let missing = HerdrCli::new(fake.config_dir().join("not-installed").join("herdr"));

    let observed = observe_with(missing, &fake, quick_retries(10));

    let presented = observed.wait_for("a refusal", is_refused);
    assert!(matches!(
        presented.connection,
        ConnectionStatus::Refused(Refusal::ExecutableMissing { .. })
    ));
}

#[test]
fn dropping_the_agents_closes_their_subscription() {
    let fake = fake();
    let session = fake.start_session("default", fleet(AgentState::Working));
    let observed = observe(&fake, ReconnectPolicy::default());
    observed.wait_for_state("w1:p1", AgentState::Working);
    let updates = Arc::clone(&observed.updates);

    drop(observed);

    eventually("the subscription to close", || {
        (session.subscriber_count() == 0).then_some(())
    });
    session.set_script(fleet(AgentState::Blocked));
    session.emit_event("pane_updated", serde_json::json!({}));
    thread::sleep(Duration::from_millis(50));
    assert_eq!(
        updates
            .lock()
            .last()
            .and_then(|presented| presented.agent(&"w1:p1".into()).map(|agent| agent.state)),
        Some(presented_state(AgentState::Working))
    );
}

#[test]
fn an_agent_resolves_to_a_jump_target_in_its_workspace_herdr_tab_and_pane() {
    let fake = fake();
    let session = fake.start_session("default", fleet(AgentState::Working));
    let observed = observe(&fake, ReconnectPolicy::default());
    let presented = observed.wait_for_connected("the agents", |p| p.agents.len() == 2);

    let presented_session = authoritative(&fake);
    let jump = presented.agents[1]
        .jump_target(&presented_session)
        .expect("the agent's pane is presented");

    assert_eq!(jump.workspace, WorkspaceId::from("w1"));
    assert_eq!(jump.herdr_tab, HerdrTabId::from("w1:t2"));
    assert_eq!(jump.pane.as_str(), "w1:p3");
    let moved_focus = session
        .served_methods()
        .iter()
        .any(|method| method.ends_with(".focus") || method.ends_with(".focused"));
    assert!(!moved_focus, "{:?}", session.served_methods());
}

#[test]
fn an_agent_whose_pane_the_session_does_not_present_has_no_jump_target() {
    let fake = fake();
    let session = fake.start_session("default", fleet(AgentState::Working));
    let observed = observe(&fake, ReconnectPolicy::default());
    let presented = observed.wait_for_connected("the agents", |p| p.agents.len() == 2);
    let stale_session = authoritative(&fake);

    // The session mirror is read before the agent list, so either can be ahead of the other.
    session.set_script(
        SessionScript::new()
            .workspace_with_tabs(
                "api",
                [
                    TabScript::new("server", PaneTree::pane()),
                    TabScript::new("logs", PaneTree::pane()),
                ],
            )
            .workspace("web"),
    );
    let current_session = authoritative(&fake);
    let logs_agent = &presented.agents[1];

    assert!(logs_agent.jump_target(&stale_session).is_some());
    assert!(logs_agent.jump_target(&current_session).is_none());
    assert!(logs_agent.location(&current_session).is_some());
    assert!(
        presented.agents[0]
            .jump_target(&PresentedSession::connecting())
            .is_none()
    );
}
