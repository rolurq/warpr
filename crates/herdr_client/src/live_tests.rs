use std::sync::Arc;
use std::thread;
use std::time::Duration;

use herdr_fake::{
    FakeHerdr, FakeSession, HerdrStatus, PaneTree, SessionScript, SplitDirection, TabScript,
};
use instant::Instant;
use parking_lot::Mutex;
use serde_json::json;

use super::*;
use crate::{ConnectionFailure, HerdrCli, PaneLayout, Refusal, SessionName, present_session};

const WAIT_LIMIT: Duration = Duration::from_secs(10);

/// Retries quickly so tests spend their time on behaviour rather than on backoff.
fn quick_retries(disconnected_after: u32) -> ReconnectPolicy {
    ReconnectPolicy {
        initial_backoff: Duration::from_millis(5),
        max_backoff: Duration::from_millis(20),
        disconnected_after,
    }
}

/// A Herdr whose executable and server are compatible, so connecting is not refused.
fn fake() -> FakeHerdr {
    let mut fake = FakeHerdr::new();
    fake.script_status(&HerdrStatus::compatible());
    fake
}

fn cli(fake: &FakeHerdr) -> HerdrCli {
    HerdrCli::new(fake.binary_path())
}

/// What Herdr itself reports, read the way a one-off connection reads it.
fn authoritative(fake: &FakeHerdr) -> PresentedSession {
    present_session(&cli(fake), &target(fake))
}

fn target(fake: &FakeHerdr) -> SessionTarget {
    SessionTarget::new(
        &fake.config_dir(),
        SessionName::new("default").expect("valid name"),
    )
}

fn workspaces(labels: &[&str]) -> SessionScript {
    labels.iter().fold(SessionScript::new(), |script, label| {
        script.workspace(label)
    })
}

fn labels(presented: &PresentedSession) -> Vec<&str> {
    presented
        .workspaces
        .iter()
        .map(|workspace| workspace.label.as_str())
        .collect()
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

/// A [`LiveSession`] together with everything it presented.
struct Observed {
    live: LiveSession,
    updates: Arc<Mutex<Vec<(Instant, PresentedSession)>>>,
}

fn observe(fake: &FakeHerdr, policy: ReconnectPolicy) -> Observed {
    observe_with(cli(fake), fake, policy)
}

fn observe_with(cli: HerdrCli, fake: &FakeHerdr, policy: ReconnectPolicy) -> Observed {
    let updates = Arc::new(Mutex::new(Vec::new()));
    let live = LiveSession::start(cli, target(fake), policy, {
        let updates = Arc::clone(&updates);
        move |presented| updates.lock().push((Instant::now(), presented))
    });
    Observed { live, updates }
}

impl Observed {
    fn latest(&self) -> PresentedSession {
        self.updates
            .lock()
            .last()
            .map_or_else(PresentedSession::connecting, |(_, presented)| {
                presented.clone()
            })
    }

    fn history(&self) -> Vec<PresentedSession> {
        self.updates
            .lock()
            .iter()
            .map(|(_, presented)| presented.clone())
            .collect()
    }

    fn wait_for(
        &self,
        expectation: &str,
        mut matches: impl FnMut(&PresentedSession) -> bool,
    ) -> PresentedSession {
        eventually(expectation, || {
            let latest = self.latest();
            matches(&latest).then_some(latest)
        })
    }

    fn wait_for_labels(&self, expected: &[&str]) -> PresentedSession {
        self.wait_for(&format!("workspaces {expected:?}"), |presented| {
            presented.connection == ConnectionStatus::Connected && labels(presented) == expected
        })
    }

    fn first_update_where(&self, matches: impl Fn(&PresentedSession) -> bool) -> Option<Instant> {
        self.updates
            .lock()
            .iter()
            .find(|(_, presented)| matches(presented))
            .map(|(at, _)| *at)
    }
}

fn is_refused(presented: &PresentedSession) -> bool {
    matches!(presented.connection, ConnectionStatus::Refused(_))
}

fn is_reconnecting(presented: &PresentedSession) -> bool {
    matches!(presented.connection, ConnectionStatus::Reconnecting(_))
}

fn is_disconnected(presented: &PresentedSession) -> bool {
    matches!(presented.connection, ConnectionStatus::Disconnected(_))
}

/// Tells Herdr's subscribers that `data`, whatever it claims, happened.
fn announce(session: &FakeSession, event: &str) {
    session.emit_event(event, json!({ "type": event }));
}

#[test]
fn the_session_is_connecting_until_the_first_snapshot_arrives() {
    let fake = fake();
    let session = fake.start_session("default", workspaces(&["api"]));
    let pause = session.pause_snapshots();

    let observed = observe(&fake, ReconnectPolicy::default());
    pause.wait_until_requested();

    assert_eq!(observed.latest(), PresentedSession::connecting());
    drop(pause);
    observed.wait_for_labels(&["api"]);
}

#[test]
fn workspaces_created_closed_and_renamed_elsewhere_appear_without_a_refresh() {
    let fake = fake();
    let session = fake.start_session("default", workspaces(&["api", "web"]));
    let observed = observe(&fake, ReconnectPolicy::default());
    observed.wait_for_labels(&["api", "web"]);

    session.set_script(workspaces(&["api", "web", "docs"]));
    announce(&session, "workspace_created");
    observed.wait_for_labels(&["api", "web", "docs"]);

    session.set_script(workspaces(&["api", "backend", "docs"]));
    announce(&session, "workspace_renamed");
    observed.wait_for_labels(&["api", "backend", "docs"]);

    session.set_script(workspaces(&["api", "backend"]));
    announce(&session, "workspace_closed");
    observed.wait_for_labels(&["api", "backend"]);
}

#[test]
fn a_focus_change_made_elsewhere_is_presented() {
    let fake = fake();
    let session = fake.start_session("default", workspaces(&["api", "web"]).focused("api"));
    let observed = observe(&fake, ReconnectPolicy::default());
    observed.wait_for("api focused", |presented| {
        presented.focused_workspace_index() == Some(0)
    });

    session.set_script(workspaces(&["api", "web"]).focused("web"));
    announce(&session, "workspace_focused");

    observed.wait_for("web focused", |presented| {
        presented.focused_workspace_index() == Some(1)
    });
}

#[test]
fn an_events_payload_is_never_taken_as_the_new_state() {
    let fake = fake();
    let session = fake.start_session("default", workspaces(&["api"]));
    let observed = observe(&fake, ReconnectPolicy::default());
    observed.wait_for_labels(&["api"]);

    // The payloads claim a rename and a new workspace that Herdr's own snapshot contradicts.
    session.set_script(workspaces(&["api-v2"]));
    session.emit_event(
        "workspace_renamed",
        json!({ "type": "workspace_renamed", "workspace_id": "w1", "label": "bogus" }),
    );
    session.emit_event(
        "workspace_created",
        json!({
            "type": "workspace_created",
            "workspace": { "workspace_id": "w9", "label": "phantom" },
        }),
    );

    let presented = observed.wait_for_labels(&["api-v2"]);
    assert_eq!(presented, authoritative(&fake));
}

#[test]
fn every_lifecycle_event_that_can_change_a_snapshot_is_subscribed_to() {
    // Herdr's event vocabulary less `pane_output_changed`, which has no subscription type, and
    // the pane-scoped events, which Herdr only accepts for one named pane.
    const EVENTS: [&str; 24] = [
        "workspace_created",
        "workspace_updated",
        "workspace_metadata_updated",
        "workspace_closed",
        "workspace_renamed",
        "workspace_moved",
        "workspace_reordered",
        "workspace_focused",
        "worktree_created",
        "worktree_opened",
        "worktree_removed",
        "tab_created",
        "tab_closed",
        "tab_renamed",
        "tab_moved",
        "tab_focused",
        "pane_created",
        "pane_closed",
        "pane_updated",
        "pane_focused",
        "pane_moved",
        "pane_exited",
        "pane_agent_detected",
        "layout_updated",
    ];
    let fake = fake();
    let session = fake.start_session("default", workspaces(&["api"]));
    let observed = observe(&fake, ReconnectPolicy::default());
    observed.wait_for_labels(&["api"]);

    let unsubscribed: Vec<_> = EVENTS
        .into_iter()
        .filter(|event| session.emit_event(event, json!({})) == 0)
        .collect();

    assert_eq!(unsubscribed, Vec::<&str>::new());
}

#[test]
fn lost_events_are_recovered_by_taking_a_fresh_snapshot() {
    let fake = fake();
    let session = fake.start_session("default", workspaces(&["api", "web"]));
    let observed = observe(&fake, ReconnectPolicy::default());
    observed.wait_for_labels(&["api", "web"]);

    // The change is never announced: only the lost-events error can lead the client to it.
    session.set_script(workspaces(&["web", "docs"]).focused("docs"));
    session.lose_events();

    let presented = observed.wait_for_labels(&["web", "docs"]);
    assert_eq!(presented.focused_workspace_index(), Some(1));
    assert_eq!(presented, authoritative(&fake));
    assert!(
        observed
            .history()
            .iter()
            .all(|presented| !is_reconnecting(presented)),
        "a lost-events resync reported the connection as down"
    );
}

#[test]
fn events_keep_arriving_after_a_lost_events_resync() {
    let fake = fake();
    let session = fake.start_session("default", workspaces(&["api"]));
    let observed = observe(&fake, ReconnectPolicy::default());
    observed.wait_for_labels(&["api"]);
    session.lose_events();
    eventually("the client to subscribe again", || {
        (session.subscriber_count() == 1).then_some(())
    });

    session.set_script(workspaces(&["api", "web"]));
    announce(&session, "workspace_created");

    observed.wait_for_labels(&["api", "web"]);
}

#[test]
fn lost_events_while_subscribing_are_retried_until_a_subscription_holds() {
    let fake = fake();
    let session = fake.start_session("default", workspaces(&["api"]));
    session.lose_events_on_next_subscriptions(2);

    let observed = observe(&fake, quick_retries(10));

    observed.wait_for_labels(&["api"]);
    assert_eq!(observed.latest(), authoritative(&fake));
}

#[test]
fn a_dropped_connection_is_retried_and_the_state_reconciled() {
    let fake = fake();
    let session = fake.start_session("default", workspaces(&["api"]));
    let observed = observe(&fake, quick_retries(10));
    observed.wait_for_labels(&["api"]);

    session.set_script(workspaces(&["api", "web"]));
    session.drop_connections();

    let presented = observed.wait_for_labels(&["api", "web"]);
    assert_eq!(presented, authoritative(&fake));
    let history = observed.history();
    let went_down = history.iter().position(is_reconnecting);
    assert!(
        went_down.is_some(),
        "connection health never showed the drop: {history:?}"
    );
}

#[test]
fn a_session_that_goes_away_keeps_its_last_state_while_reconnecting() {
    let fake = fake();
    let session = fake.start_session("default", workspaces(&["api", "web"]));
    let observed = observe(&fake, quick_retries(1000));
    observed.wait_for_labels(&["api", "web"]);

    session.go_offline();

    let presented = observed.wait_for("reconnecting", |presented| {
        matches!(
            presented.connection,
            ConnectionStatus::Reconnecting(ConnectionFailure::SocketUnreachable { .. })
        )
    });
    assert_eq!(labels(&presented), ["api", "web"]);

    session.set_script(workspaces(&["docs"]));
    session.go_online();

    observed.wait_for_labels(&["docs"]);
}

#[test]
fn a_session_that_is_not_running_yet_is_picked_up_once_it_starts() {
    let fake = fake();
    let session = fake.start_session("default", workspaces(&["api"]));
    session.go_offline();

    let observed = observe(&fake, quick_retries(1000));
    observed.wait_for("reconnecting", is_reconnecting);
    assert!(observed.latest().workspaces.is_empty());

    session.go_online();

    observed.wait_for_labels(&["api"]);
}

#[test]
fn a_session_that_stays_down_is_reported_as_disconnected_and_still_recovers() {
    let fake = fake();
    let session = fake.start_session("default", workspaces(&["api"]));
    let policy = ReconnectPolicy {
        initial_backoff: Duration::from_millis(30),
        max_backoff: Duration::from_millis(100),
        disconnected_after: 3,
    };
    let observed = observe(&fake, policy);
    observed.wait_for_labels(&["api"]);

    session.go_offline();

    let presented = observed.wait_for("disconnected", is_disconnected);
    assert_eq!(labels(&presented), ["api"]);
    let reconnecting_since = observed
        .first_update_where(is_reconnecting)
        .expect("reconnecting was presented");
    let disconnected_at = observed
        .first_update_where(is_disconnected)
        .expect("disconnected was presented");
    // Two backoffs separate the three attempts: 30 ms, then 60 ms.
    assert!(disconnected_at - reconnecting_since >= Duration::from_millis(90));

    session.set_script(workspaces(&["web"]));
    session.go_online();

    observed.wait_for_labels(&["web"]);
}

#[test]
fn asking_to_reconnect_cuts_a_backoff_short() {
    let fake = fake();
    let session = fake.start_session("default", workspaces(&["api"]));
    let policy = ReconnectPolicy {
        initial_backoff: Duration::from_secs(60),
        max_backoff: Duration::from_secs(60),
        disconnected_after: 100,
    };
    let observed = observe(&fake, policy);
    observed.wait_for_labels(&["api"]);
    session.go_offline();
    observed.wait_for("reconnecting", is_reconnecting);

    session.go_online();
    observed.live.reconnect();

    observed.wait_for_labels(&["api"]);
}

#[test]
fn a_snapshot_overtaken_by_an_event_is_read_again() {
    let fake = fake();
    let session = fake.start_session("default", workspaces(&["api"]));
    let pause = session.pause_snapshots();
    let observed = observe(&fake, ReconnectPolicy::default());
    pause.wait_until_requested();

    // The held response already describes `["api"]`; Herdr moves on before it is delivered.
    session.set_script(workspaces(&["api", "web"]));
    announce(&session, "workspace_created");
    drop(pause);

    observed.wait_for_labels(&["api", "web"]);
}

#[test]
fn dropping_the_session_closes_its_subscription() {
    let fake = fake();
    let session = fake.start_session("default", workspaces(&["api"]));
    let observed = observe(&fake, ReconnectPolicy::default());
    observed.wait_for_labels(&["api"]);
    let updates = Arc::clone(&observed.updates);

    drop(observed);

    eventually("the subscription to close", || {
        (session.subscriber_count() == 0).then_some(())
    });
    session.set_script(workspaces(&["api", "web"]));
    announce(&session, "workspace_created");
    thread::sleep(Duration::from_millis(50));
    assert_eq!(updates.lock().len(), 1);
}

#[test]
fn an_incompatible_herdr_is_refused_and_nothing_of_the_session_is_presented() {
    let mut fake = fake();
    fake.script_status(&HerdrStatus::compatible().server_protocol(21));
    let session = fake.start_session("default", workspaces(&["api"]));

    let observed = observe(&fake, quick_retries(10));

    let presented = observed.wait_for("a refusal", is_refused);
    assert!(presented.workspaces.is_empty());
    assert_eq!(session.subscriber_count(), 0);
}

#[test]
fn a_missing_herdr_executable_is_refused_by_the_live_session() {
    let fake = fake();
    let _session = fake.start_session("default", workspaces(&["api"]));
    let missing = HerdrCli::new(fake.config_dir().join("not-installed").join("herdr"));

    let observed = observe_with(missing, &fake, quick_retries(10));

    let presented = observed.wait_for("a refusal", is_refused);
    assert!(matches!(
        presented.connection,
        ConnectionStatus::Refused(Refusal::ExecutableMissing { .. })
    ));
}

#[test]
fn a_server_restarted_as_an_incompatible_release_is_refused_until_it_is_compatible_again() {
    let mut fake = fake();
    let session = fake.start_session("default", workspaces(&["api"]));
    let observed = observe(&fake, quick_retries(10_000));
    observed.wait_for_labels(&["api"]);

    session.go_offline();
    fake.script_status(&HerdrStatus::compatible().server_protocol(21));
    session.set_script(workspaces(&["web"]));
    session.go_online();

    let refused = observed.wait_for("a refusal", is_refused);
    assert!(refused.workspaces.is_empty());

    fake.script_status(&HerdrStatus::compatible());

    observed.wait_for_labels(&["web"]);
}

#[test]
fn backoff_doubles_with_each_failure_up_to_the_maximum() {
    let policy = ReconnectPolicy {
        initial_backoff: Duration::from_millis(250),
        max_backoff: Duration::from_secs(2),
        disconnected_after: 10,
    };

    let backoffs: Vec<_> = (1..=6).map(|failures| policy.backoff(failures)).collect();

    assert_eq!(
        backoffs,
        [
            Duration::from_millis(250),
            Duration::from_millis(500),
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(2),
            Duration::from_secs(2),
        ]
    );
    assert_eq!(policy.backoff(u32::MAX), Duration::from_secs(2));
}

/// One thing that can happen to a session. Every variant ends in something that lets a client
/// that is paying attention notice, because Herdr does not make silent changes either.
#[derive(Clone, Copy, Debug)]
enum Step {
    Announced,
    AnnouncedWithMisleadingPayload,
    Unannounced { then: Interruption },
}

#[derive(Clone, Copy, Debug)]
enum Interruption {
    LostEvents,
    DroppedConnections,
    Restart,
}

/// A small deterministic generator, so a failing sequence can be replayed from its seed.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, bound: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % bound
    }
}

fn random_script(rng: &mut Lcg) -> SessionScript {
    const LABELS: [&str; 5] = ["api", "web", "docs", "ops", "infra"];
    let mut labels = LABELS.to_vec();
    let count = rng.next(labels.len() + 1);
    let mut script = SessionScript::new();
    let mut chosen = Vec::new();
    for _ in 0..count {
        let label = labels.remove(rng.next(labels.len()));
        chosen.push(label);
        script = script.workspace(label);
    }
    if !chosen.is_empty() {
        script = script.focused(chosen[rng.next(chosen.len())]);
    }
    script
}

fn random_step(rng: &mut Lcg) -> Step {
    match rng.next(5) {
        0 => Step::Announced,
        1 => Step::AnnouncedWithMisleadingPayload,
        2 => Step::Unannounced {
            then: Interruption::LostEvents,
        },
        3 => Step::Unannounced {
            then: Interruption::DroppedConnections,
        },
        _ => Step::Unannounced {
            then: Interruption::Restart,
        },
    }
}

/// Plays 40 random steps against a session, without waiting between them, for each seed, and
/// returns where each run converged.
fn assert_sequences_converge(
    mut next_script: impl FnMut(&mut Lcg) -> SessionScript,
) -> Vec<PresentedSession> {
    let mut converged = Vec::new();
    for seed in 1..=6 {
        let mut rng = Lcg(seed);
        let fake = fake();
        let session = fake.start_session("default", workspaces(&["api"]));
        let observed = observe(&fake, quick_retries(10_000));
        observed.wait_for_labels(&["api"]);

        let mut steps = Vec::new();
        for _ in 0..40 {
            let step = random_step(&mut rng);
            steps.push(step);
            session.set_script(next_script(&mut rng));
            match step {
                Step::Announced => announce(&session, "workspace_updated"),
                Step::AnnouncedWithMisleadingPayload => {
                    session.emit_event(
                        "workspace_renamed",
                        json!({ "workspace_id": "w1", "label": "misleading" }),
                    );
                }
                Step::Unannounced {
                    then: Interruption::LostEvents,
                } => session.lose_events(),
                Step::Unannounced {
                    then: Interruption::DroppedConnections,
                } => session.drop_connections(),
                Step::Unannounced {
                    then: Interruption::Restart,
                } => {
                    session.go_offline();
                    session.go_online();
                }
            }
        }

        let presented = observed.wait_for(
            &format!("seed {seed} to converge after {steps:?}"),
            |presented| *presented == authoritative(&fake),
        );
        assert_eq!(presented.connection, ConnectionStatus::Connected);
        converged.push(presented);
    }
    converged
}

#[test]
fn after_any_scripted_sequence_the_presented_state_equals_herdrs_snapshot() {
    assert_sequences_converge(random_script);
}

fn random_tree(rng: &mut Lcg, depth: usize) -> (PaneTree, usize) {
    if depth == 0 || rng.next(3) == 0 {
        return (PaneTree::pane(), 1);
    }
    let (first, first_panes) = random_tree(rng, depth - 1);
    let (second, second_panes) = random_tree(rng, depth - 1);
    let direction = if rng.next(2) == 0 {
        SplitDirection::Right
    } else {
        SplitDirection::Down
    };
    let ratio = [0.25, 0.5, 0.75][rng.next(3)];
    (
        PaneTree::split(direction, ratio, first, second),
        first_panes + second_panes,
    )
}

fn random_tab(rng: &mut Lcg, label: &str) -> TabScript {
    let (tree, panes) = random_tree(rng, 2);
    let tab = TabScript::new(label, tree).focused_pane(rng.next(panes));
    if rng.next(4) == 0 { tab.zoomed() } else { tab }
}

fn random_structured_script(rng: &mut Lcg) -> SessionScript {
    let workspace_count = 1 + rng.next(3);
    ["api", "web", "docs"].iter().take(workspace_count).fold(
        SessionScript::new(),
        |script, label| {
            let tab_count = 1 + rng.next(3);
            let tabs: Vec<_> = (0..tab_count)
                .map(|index| random_tab(rng, &format!("tab{index}")))
                .collect();
            script.workspace_with_tabs(label, tabs)
        },
    )
}

#[test]
fn after_any_scripted_sequence_over_tabs_panes_and_layout_the_presented_state_equals_herdrs_snapshot()
 {
    for presented in assert_sequences_converge(random_structured_script) {
        let tabs = presented.workspaces.iter().flat_map(|w| &w.herdr_tabs);
        assert!(tabs.clone().count() > 0);
        assert!(tabs.into_iter().all(|tab| tab.layout.is_some()));
    }
}

fn api_with_tabs(tabs: impl IntoIterator<Item = TabScript>) -> SessionScript {
    SessionScript::new().workspace_with_tabs("api", tabs)
}

fn two_panes(ratio: f32) -> PaneTree {
    PaneTree::split(
        SplitDirection::Right,
        ratio,
        PaneTree::pane(),
        PaneTree::pane(),
    )
}

fn api_tab_labels(presented: &PresentedSession) -> Vec<&str> {
    presented
        .workspaces
        .iter()
        .find(|workspace| workspace.label == "api")
        .map(|workspace| {
            workspace
                .herdr_tabs
                .iter()
                .map(|tab| tab.label.as_str())
                .collect()
        })
        .unwrap_or_default()
}

fn api_layout<'a>(presented: &'a PresentedSession, tab_label: &str) -> Option<&'a PaneLayout> {
    presented
        .workspaces
        .iter()
        .find(|workspace| workspace.label == "api")?
        .herdr_tabs
        .iter()
        .find(|tab| tab.label == tab_label)?
        .layout
        .as_ref()
}

#[test]
fn herdr_tabs_created_renamed_and_closed_elsewhere_appear_without_a_refresh() {
    let fake = fake();
    let session = fake.start_session(
        "default",
        api_with_tabs([TabScript::new("1", PaneTree::pane())]),
    );
    let observed = observe(&fake, ReconnectPolicy::default());
    observed.wait_for("one Herdr tab", |presented| {
        api_tab_labels(presented) == ["1"]
    });

    session.set_script(api_with_tabs([
        TabScript::new("1", PaneTree::pane()),
        TabScript::new("logs", PaneTree::pane()),
    ]));
    announce(&session, "tab_created");
    observed.wait_for("a second Herdr tab", |presented| {
        api_tab_labels(presented) == ["1", "logs"]
    });

    session.set_script(api_with_tabs([
        TabScript::new("1", PaneTree::pane()),
        TabScript::new("build", PaneTree::pane()),
    ]));
    announce(&session, "tab_renamed");
    observed.wait_for("the renamed Herdr tab", |presented| {
        api_tab_labels(presented) == ["1", "build"]
    });

    session.set_script(api_with_tabs([TabScript::new("1", PaneTree::pane())]));
    announce(&session, "tab_closed");
    observed.wait_for("the Herdr tab gone", |presented| {
        api_tab_labels(presented) == ["1"]
    });
}

#[test]
fn panes_split_and_closed_elsewhere_appear_without_a_refresh() {
    let fake = fake();
    let session = fake.start_session(
        "default",
        api_with_tabs([TabScript::new("1", PaneTree::pane())]),
    );
    let observed = observe(&fake, ReconnectPolicy::default());
    observed.wait_for("one pane", |presented| {
        api_layout(presented, "1").is_some_and(|layout| layout.panes.len() == 1)
    });

    session.set_script(api_with_tabs([TabScript::new("1", two_panes(0.5))]));
    announce(&session, "pane_created");
    observed.wait_for("two panes", |presented| {
        api_layout(presented, "1").is_some_and(|layout| layout.panes.len() == 2)
    });

    session.set_script(api_with_tabs([TabScript::new("1", PaneTree::pane())]));
    announce(&session, "pane_closed");
    observed.wait_for("one pane again", |presented| {
        api_layout(presented, "1").is_some_and(|layout| layout.panes.len() == 1)
    });
}

#[test]
fn layout_changes_made_in_another_client_appear_live() {
    let fake = fake();
    let session = fake.start_session(
        "default",
        api_with_tabs([TabScript::new("1", two_panes(0.5))]),
    );
    let observed = observe(&fake, ReconnectPolicy::default());
    let first_pane_width = |presented: &PresentedSession| {
        api_layout(presented, "1").map(|layout| layout.panes[0].rect.width)
    };
    observed.wait_for("an even split", |presented| {
        first_pane_width(presented) == Some(60)
    });

    session.set_script(api_with_tabs([TabScript::new("1", two_panes(0.75))]));
    announce(&session, "layout_updated");
    observed.wait_for("a resized split", |presented| {
        first_pane_width(presented) == Some(90)
    });

    session.set_script(api_with_tabs([
        TabScript::new("1", two_panes(0.75)).zoomed()
    ]));
    announce(&session, "layout_updated");
    let zoomed = observed.wait_for("a zoomed pane", |presented| {
        api_layout(presented, "1").is_some_and(|layout| layout.zoomed)
    });
    assert_eq!(api_layout(&zoomed, "1").unwrap().frames().len(), 1);

    session.set_script(api_with_tabs([TabScript::new("1", two_panes(0.75))
        .zoomed()
        .focused_pane(1)]));
    announce(&session, "pane_focused");
    observed.wait_for("the other pane focused", |presented| {
        api_layout(presented, "1")
            .is_some_and(|layout| layout.zoomed && layout.focused_pane == layout.panes[1].id)
    });
}

#[test]
fn a_layout_change_whose_events_were_lost_is_presented_after_the_resync() {
    let fake = fake();
    let session = fake.start_session(
        "default",
        api_with_tabs([TabScript::new("1", two_panes(0.5))]),
    );
    let observed = observe(&fake, ReconnectPolicy::default());
    observed.wait_for("an even split", |presented| {
        api_layout(presented, "1").is_some_and(|layout| layout.panes[0].rect.width == 60)
    });

    session.set_script(api_with_tabs([
        TabScript::new("1", two_panes(0.25)),
        TabScript::new("logs", PaneTree::pane()),
    ]));
    session.lose_events();

    let presented = observed.wait_for("the resized split and the new Herdr tab", |presented| {
        api_tab_labels(presented) == ["1", "logs"]
            && api_layout(presented, "1").is_some_and(|layout| layout.panes[0].rect.width == 30)
    });
    assert_eq!(presented, authoritative(&fake));
}
