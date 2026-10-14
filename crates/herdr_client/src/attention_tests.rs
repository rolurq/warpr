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
use crate::notifications::{AgentNotification, NotificationKind};
use crate::{
    AgentState, ConnectionStatus, HerdrCli, HerdrTabId, LiveAgents, PresentedAgent,
    ReconnectPolicy, SessionName, SessionTarget,
};

const WAIT_LIMIT: Duration = Duration::from_secs(10);
const WINDOW: ViewerId = ViewerId(1);
const OTHER_WINDOW: ViewerId = ViewerId(2);

fn fake() -> FakeHerdr {
    let mut fake = FakeHerdr::new();
    fake.script_status(&HerdrStatus::compatible());
    fake
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
/// panes `w1:p2` and `w1:p3`). Herdr has a named `claude` in `w1:p1` in `builder` state. Workspace
/// `web` (`w2`) is empty.
fn fleet(builder: ScriptedState) -> SessionScript {
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
                ),
            ],
        )
        .workspace("web")
}

fn builder(state: ScriptedState) -> PresentedAgent {
    PresentedAgent {
        pane: "w1:p1".into(),
        workspace: "w1".into(),
        herdr_tab: HerdrTabId::from("w1:t1"),
        kind: Some("claude".to_owned()),
        name: Some("builder".to_owned()),
        state: presented_state(state),
    }
}

fn raised(kind: NotificationKind, agent: PresentedAgent) -> AgentNotification {
    AgentNotification { kind, agent }
}

/// A front-end following the agents of a fake session through [`AgentAttention`], recording what
/// it was told to do.
struct Following {
    _live: LiveAgents,
    attention: Arc<Mutex<AgentAttention>>,
    told: Arc<Mutex<Vec<AttentionChanges>>>,
    connected: Arc<Mutex<Option<PresentedAgentsSummary>>>,
}

#[derive(Clone)]
struct PresentedAgentsSummary {
    connection: ConnectionStatus,
    builder: Option<AgentState>,
}

fn follow(fake: &FakeHerdr) -> Following {
    let attention = Arc::new(Mutex::new(AgentAttention::new()));
    let told = Arc::new(Mutex::new(Vec::new()));
    let latest = Arc::new(Mutex::new(None));
    let target = SessionTarget::new(
        &fake.config_dir(),
        SessionName::new("default").expect("valid name"),
    );
    let live = LiveAgents::start(
        HerdrCli::new(fake.binary_path()),
        target,
        ReconnectPolicy::default(),
        {
            let attention = Arc::clone(&attention);
            let told = Arc::clone(&told);
            let latest = Arc::clone(&latest);
            move |presented| {
                *latest.lock() = Some(PresentedAgentsSummary {
                    connection: presented.connection.clone(),
                    builder: presented.agent(&"w1:p1".into()).map(|agent| agent.state),
                });
                let changes = attention.lock().observe(&presented);
                told.lock().push(changes);
            }
        },
    );
    Following {
        _live: live,
        attention,
        told,
        connected: latest,
    }
}

impl Following {
    fn wait_for_builder(&self, state: ScriptedState) {
        eventually(&format!("builder to be {state:?}"), || {
            let latest = self.connected.lock().clone()?;
            (latest.connection == ConnectionStatus::Connected
                && latest.builder == Some(presented_state(state)))
            .then_some(())
        });
    }

    /// Everything the front-end was told to do so far, in order.
    fn told(&self) -> AttentionChanges {
        let told = self.told.lock();
        AttentionChanges {
            notifications: told
                .iter()
                .flat_map(|changes| changes.notifications.clone())
                .collect(),
            desktop: told
                .iter()
                .flat_map(|changes| changes.desktop.clone())
                .collect(),
        }
    }

    fn record(&self, changes: AttentionChanges) {
        self.told.lock().push(changes);
    }

    fn show(&self, viewer: ViewerId, workspace: &str, panes: &[&str]) {
        let changes = self.attention.lock().show(
            viewer,
            workspace.into(),
            panes.iter().map(|pane| PaneId::from(*pane)),
        );
        self.record(changes);
    }

    fn select(&self, viewer: ViewerId, workspace: Option<&str>) {
        let changes = self
            .attention
            .lock()
            .select(viewer, workspace.map(Into::into));
        self.record(changes);
    }

    fn focus(&self, focused: Option<ViewerId>) {
        let changes = self.attention.lock().focus(focused);
        self.record(changes);
    }
}

fn wait_for_status_subscriptions(session: &FakeSession) {
    let panes = ["w1:p1", "w1:p2", "w1:p3", "w2:p1"];
    eventually("status subscriptions", || {
        (session.agent_status_subscriptions() == panes).then_some(())
    });
}

/// A connected session whose builder is working, followed by a front-end whose only window shows
/// the pane `w1:p1` of workspace `w1`.
fn working_builder(fake: &FakeHerdr) -> (FakeSession, Following) {
    let session = fake.start_session("default", fleet(ScriptedState::Working));
    let following = follow(fake);
    following.wait_for_builder(ScriptedState::Working);
    wait_for_status_subscriptions(&session);
    following.show(WINDOW, "w1", &["w1:p1"]);
    following.select(WINDOW, Some("w1"));
    (session, following)
}

fn change_builder(session: &FakeSession, following: &Following, state: ScriptedState) {
    session.set_script(fleet(state));
    session.emit_agent_status_changed("w1:p1", "claude", state);
    following.wait_for_builder(state);
}

#[test]
fn a_blocked_agent_raises_a_desktop_notification_while_the_application_is_not_focused() {
    let fake = fake();
    let (session, following) = working_builder(&fake);
    following.focus(None);

    change_builder(&session, &following, ScriptedState::Blocked);

    let notification = raised(NotificationKind::Blocked, builder(ScriptedState::Blocked));
    let told = following.told();
    assert_eq!(
        told.notifications,
        [NotificationChange::Raised(notification.clone())]
    );
    assert_eq!(told.desktop, [DesktopAction::Raise(notification)]);
}

#[test]
fn a_blocked_agent_raises_only_in_app_notifications_while_the_application_is_focused() {
    let fake = fake();
    let (session, following) = working_builder(&fake);
    following.focus(Some(WINDOW));

    change_builder(&session, &following, ScriptedState::Blocked);

    let told = following.told();
    assert_eq!(told.notifications.len(), 1);
    assert_eq!(told.desktop, []);
}

#[test]
fn finishing_in_a_pane_the_focused_window_shows_notifies_nobody() {
    let fake = fake();
    let (session, following) = working_builder(&fake);
    following.focus(Some(WINDOW));

    change_builder(&session, &following, ScriptedState::Done);

    assert_eq!(following.told(), AttentionChanges::default());
}

#[test]
fn finishing_in_a_pane_shown_by_an_unfocused_window_raises_a_desktop_notification() {
    let fake = fake();
    let (session, following) = working_builder(&fake);
    following.focus(None);

    change_builder(&session, &following, ScriptedState::Done);

    let notification = raised(NotificationKind::Finished, builder(ScriptedState::Done));
    assert_eq!(
        following.told().desktop,
        [DesktopAction::Raise(notification)]
    );
}

#[test]
fn a_desktop_notification_is_withdrawn_when_the_agent_leaves_the_state_that_raised_it() {
    let fake = fake();
    let (session, following) = working_builder(&fake);
    following.focus(None);
    change_builder(&session, &following, ScriptedState::Blocked);

    change_builder(&session, &following, ScriptedState::Working);

    let told = following.told();
    assert_eq!(told.desktop.len(), 2);
    assert_eq!(told.desktop[1], DesktopAction::Withdraw("w1:p1".into()));
}

#[test]
fn an_agent_blocked_after_finishing_replaces_the_desktop_notification() {
    let fake = fake();
    let (session, following) = working_builder(&fake);
    following.focus(None);
    change_builder(&session, &following, ScriptedState::Done);

    change_builder(&session, &following, ScriptedState::Blocked);

    let desktop = following.told().desktop;
    assert_eq!(
        desktop[1..],
        [
            DesktopAction::Withdraw("w1:p1".into()),
            DesktopAction::Raise(raised(
                NotificationKind::Blocked,
                builder(ScriptedState::Blocked)
            )),
        ]
    );
}

#[test]
fn nothing_is_withdrawn_that_was_never_raised_on_the_desktop() {
    let fake = fake();
    let (session, following) = working_builder(&fake);
    following.focus(Some(WINDOW));
    change_builder(&session, &following, ScriptedState::Blocked);

    change_builder(&session, &following, ScriptedState::Working);

    assert_eq!(following.told().desktop, []);
}

#[test]
fn coming_back_to_the_pane_withdraws_its_desktop_notification() {
    let fake = fake();
    let (session, following) = working_builder(&fake);
    following.focus(None);
    change_builder(&session, &following, ScriptedState::Blocked);

    following.focus(Some(WINDOW));

    assert_eq!(
        following.told().desktop[1..],
        [DesktopAction::Withdraw("w1:p1".into())]
    );
}

#[test]
fn coming_back_to_another_pane_leaves_the_desktop_notification() {
    let fake = fake();
    let (session, following) = working_builder(&fake);
    following.show(WINDOW, "w1", &["w1:p2"]);
    following.focus(None);
    change_builder(&session, &following, ScriptedState::Blocked);

    following.focus(Some(WINDOW));

    assert_eq!(following.told().desktop.len(), 1);
}

#[test]
fn switching_to_another_herdr_tab_makes_the_agent_unseen_and_back_makes_it_seen() {
    let fake = fake();
    let (session, following) = working_builder(&fake);
    following.focus(Some(WINDOW));

    following.show(WINDOW, "w1", &["w1:p2", "w1:p3"]);
    change_builder(&session, &following, ScriptedState::Done);
    assert_eq!(
        following.told().notifications,
        [NotificationChange::Raised(raised(
            NotificationKind::Finished,
            builder(ScriptedState::Done)
        ))]
    );

    following.show(WINDOW, "w1", &["w1:p1"]);
    assert_eq!(
        following.told().notifications[1..],
        [NotificationChange::Retired {
            pane: "w1:p1".into(),
            kind: NotificationKind::Finished
        }]
    );
}

#[test]
fn switching_to_another_workspace_makes_its_panes_unseen() {
    let fake = fake();
    let (session, following) = working_builder(&fake);
    following.focus(Some(WINDOW));

    following.select(WINDOW, Some("w2"));
    change_builder(&session, &following, ScriptedState::Done);

    assert_eq!(following.told().notifications.len(), 1);

    following.select(WINDOW, Some("w1"));
    assert_eq!(following.told().notifications.len(), 2);
}

#[test]
fn a_window_showing_no_workspace_sees_no_panes() {
    let fake = fake();
    let (session, following) = working_builder(&fake);
    following.focus(Some(WINDOW));

    following.select(WINDOW, None);
    change_builder(&session, &following, ScriptedState::Done);

    assert_eq!(following.told().notifications.len(), 1);
}

#[test]
fn seen_state_does_not_depend_on_which_window_reported_last() {
    let fake = fake();
    let (session, following) = working_builder(&fake);
    following.show(OTHER_WINDOW, "w1", &["w1:p2"]);
    following.select(OTHER_WINDOW, Some("w1"));

    following.focus(Some(WINDOW));
    change_builder(&session, &following, ScriptedState::Done);
    assert_eq!(
        following.told(),
        AttentionChanges::default(),
        "the focused window shows the pane, whatever the other window reported after it"
    );

    following.focus(Some(OTHER_WINDOW));
    change_builder(&session, &following, ScriptedState::Working);
    change_builder(&session, &following, ScriptedState::Done);
    assert_eq!(
        following.told().notifications.len(),
        1,
        "the focused window no longer shows the pane"
    );
}

#[test]
fn resetting_withdraws_every_desktop_notification() {
    let fake = fake();
    let (session, following) = working_builder(&fake);
    following.focus(None);
    change_builder(&session, &following, ScriptedState::Blocked);

    let reset = following.attention.lock().reset();

    assert_eq!(reset.desktop, [DesktopAction::Withdraw("w1:p1".into())]);
}
