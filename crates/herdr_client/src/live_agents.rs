use std::collections::HashSet;
use std::sync::Arc;
use std::thread;

use serde_json::json;

use crate::agents::{PresentedAgents, fetch_agents, fetch_pane_ids};
use crate::events::{EventStream, StreamEnd, lifecycle_subscriptions};
use crate::live::{ConnectionEnd, Outcome, Pause, Shared, Wake, spawn_reader};
use crate::{
    ConnectionFailure, ConnectionStatus, HerdrCli, PaneId, ReconnectPolicy, STATUS_TIMEOUT,
    SessionTarget, compat,
};

const AGENT_STATUS_CHANGED: &str = "pane.agent_status_changed";

const PANE_NOT_FOUND: &str = "pane_not_found";

/// How many times in a row a subscription naming panes that closed since they were listed is
/// retried with a fresh list before it counts as a failed connection.
const MAX_VANISHED_PANE_RETRIES: u32 = 5;

/// The agents of a session, kept current in the background.
///
/// Herdr accepts `pane.agent_status_changed` only for one named pane, and does not announce an
/// agent's state change in any lifecycle event. So this subscribes to every pane's status next to
/// the lifecycle events, and resubscribes whenever the set of panes changes. Events only ever
/// prompt a read of `agent.list`, whose answer replaces the presented state; their payloads are
/// ignored. A dropped connection or lost events are handled as [`crate::LiveSession`] handles
/// them, and every attempt to connect is gated on the same compatibility check.
///
/// `on_update` runs on a background thread, and only when the presented state changed.
/// [`PresentedAgents::connecting`] is the state before the first call. While the connection is
/// down the last presented agents stay in place next to the connection status describing why.
pub struct LiveAgents {
    shared: Arc<Shared>,
}

impl LiveAgents {
    pub fn start(
        cli: HerdrCli,
        target: SessionTarget,
        policy: ReconnectPolicy,
        on_update: impl FnMut(PresentedAgents) + Send + 'static,
    ) -> Self {
        let shared = Arc::new(Shared::default());
        let worker = Worker {
            cli,
            target,
            policy,
            shared: Arc::clone(&shared),
            on_update: Box::new(on_update),
            presented: PresentedAgents::connecting(),
        };
        thread::Builder::new()
            .name("herdr-live-agents".to_owned())
            .spawn(move || worker.run())
            .expect("spawn Herdr live agents thread");
        Self { shared }
    }

    /// Retries now rather than after the current backoff, and starts the count of failed attempts
    /// afresh.
    pub fn reconnect(&self) {
        self.shared.request_reconnect();
    }
}

impl Drop for LiveAgents {
    fn drop(&mut self) {
        self.shared.stop();
    }
}

struct Worker {
    cli: HerdrCli,
    target: SessionTarget,
    policy: ReconnectPolicy,
    shared: Arc<Shared>,
    on_update: Box<dyn FnMut(PresentedAgents) + Send>,
    /// What `on_update` was last given.
    presented: PresentedAgents,
}

impl Worker {
    fn run(mut self) {
        let mut failures = 0;
        loop {
            let end = self.serve_connection();
            if end.established {
                failures = 0;
            }
            let status = match end.outcome {
                Outcome::Stopped => return,
                Outcome::EventsLost if end.established => continue,
                Outcome::Refused(refusal) => {
                    self.publish(PresentedAgents::refused(refusal));
                    None
                }
                Outcome::EventsLost => Some(StreamEnd::EventsLost.into_failure()),
                Outcome::Failed(failure) => Some(failure),
            };

            failures += 1;
            if let Some(failure) = status {
                self.publish_status(if failures >= self.policy.disconnected_after {
                    ConnectionStatus::Disconnected(failure)
                } else {
                    ConnectionStatus::Reconnecting(failure)
                });
            }
            match self.shared.pause(self.policy.backoff(failures)) {
                Pause::Stopped => return,
                Pause::ReconnectRequested => failures = 0,
                Pause::Elapsed => {}
            }
        }
    }

    /// Follows the agents over one connection, resubscribing for as long as it holds.
    fn serve_connection(&mut self) -> ConnectionEnd {
        self.shared.clear_reconnect_request();
        if let Err(refusal) = compat::check(&self.cli, &self.target, STATUS_TIMEOUT) {
            return ConnectionEnd::unestablished(Outcome::Refused(refusal));
        }

        let mut established = false;
        let mut vanished_pane_retries = 0;
        loop {
            let end = |outcome| ConnectionEnd {
                outcome,
                established,
            };
            let panes: HashSet<PaneId> = match fetch_pane_ids(&self.target) {
                Ok(panes) => panes.into_iter().collect(),
                Err(failure) => return end(Outcome::Failed(failure)),
            };
            let (events, shutdown) =
                match EventStream::subscribe_to(self.target.socket_path(), subscriptions(&panes)) {
                    Ok(subscription) => subscription,
                    Err(StreamEnd::EventsLost) => return end(Outcome::EventsLost),
                    Err(StreamEnd::Failed(failure)) => {
                        let pane_closed = matches!(
                            &failure,
                            ConnectionFailure::Rejected { code, .. } if code == PANE_NOT_FOUND
                        );
                        if pane_closed && vanished_pane_retries < MAX_VANISHED_PANE_RETRIES {
                            vanished_pane_retries += 1;
                            continue;
                        }
                        return end(Outcome::Failed(failure));
                    }
                };
            let Some(generation) = self.shared.begin_stream(shutdown) else {
                return end(Outcome::Stopped);
            };
            spawn_reader(Arc::clone(&self.shared), generation, events);
            vanished_pane_retries = 0;

            let outcome = self.follow_stream(&panes, &mut established);
            self.shared.end_stream();
            if let Some(outcome) = outcome {
                return ConnectionEnd {
                    outcome,
                    established,
                };
            }
        }
    }

    /// Reads the agents, then again whenever an event arrives, until the stream ends. Returns
    /// `None` when the panes Herdr has are no longer those `subscribed_panes` were subscribed
    /// for.
    fn follow_stream(
        &mut self,
        subscribed_panes: &HashSet<PaneId>,
        established: &mut bool,
    ) -> Option<Outcome> {
        loop {
            self.shared.clear_invalidation();
            let (panes, agents) = match (fetch_pane_ids(&self.target), fetch_agents(&self.target)) {
                (Ok(panes), Ok(agents)) => (panes, agents),
                (Err(failure), _) | (_, Err(failure)) => return Some(Outcome::Failed(failure)),
            };
            *established = true;
            let panes: HashSet<PaneId> = panes.into_iter().collect();
            if &panes != subscribed_panes {
                return None;
            }
            self.publish(PresentedAgents::connected(agents));

            match self.shared.wait_for_signal() {
                Wake::Invalidated => continue,
                Wake::Stopped => return Some(Outcome::Stopped),
                Wake::StreamEnded(StreamEnd::EventsLost) => return Some(Outcome::EventsLost),
                Wake::StreamEnded(StreamEnd::Failed(failure)) => {
                    return Some(Outcome::Failed(failure));
                }
            }
        }
    }

    fn publish_status(&mut self, status: ConnectionStatus) {
        let next = self.presented.clone().with_connection(status);
        self.publish(next);
    }

    fn publish(&mut self, next: PresentedAgents) {
        if next == self.presented || self.shared.is_stopped() {
            return;
        }
        self.presented = next.clone();
        (self.on_update)(next);
    }
}

/// The lifecycle events plus the status of every one of `panes`.
fn subscriptions(panes: &HashSet<PaneId>) -> Vec<serde_json::Value> {
    let mut subscriptions = lifecycle_subscriptions();
    subscriptions.extend(
        panes
            .iter()
            .map(|pane| json!({ "type": AGENT_STATUS_CHANGED, "pane_id": pane.as_str() })),
    );
    subscriptions
}

#[cfg(test)]
#[path = "live_agents_tests.rs"]
mod tests;
