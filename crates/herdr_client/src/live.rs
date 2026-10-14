use std::sync::Arc;
use std::time::Duration;
use std::{mem, thread};

use instant::Instant;
use parking_lot::{Condvar, Mutex};

use crate::events::{EventStream, StreamEnd, StreamShutdown};
use crate::{
    ConnectionFailure, ConnectionStatus, HerdrCli, PresentedSession, ReconnectPolicy, Refusal,
    STATUS_TIMEOUT, SessionTarget, compat, fetch_snapshot,
};

/// A session kept current in the background.
///
/// It subscribes to Herdr's lifecycle events and treats each one only as a cue to read a fresh
/// snapshot, which then replaces the presented state; the event's own payload is ignored. It
/// resubscribes and re-reads after Herdr reports lost events, and after a dropped connection it
/// retries with backoff and reconciles against a new snapshot. Every connection attempt, the
/// first and each retry, is gated on the compatibility check of [`crate::present_session`], so a
/// server restarted as an incompatible release is refused rather than presented. Dropping the
/// handle ends the session without waiting for its background threads.
///
/// `on_update` runs on a background thread, and only when the presented state changed.
/// [`PresentedSession::connecting`] is the state before the first call. While the connection is
/// down the last presented workspaces stay in place next to the connection status describing why.
pub struct LiveSession {
    shared: Arc<Shared>,
}

impl LiveSession {
    pub fn start(
        cli: HerdrCli,
        target: SessionTarget,
        policy: ReconnectPolicy,
        on_update: impl FnMut(PresentedSession) + Send + 'static,
    ) -> Self {
        let shared = Arc::new(Shared::default());
        let worker = Worker {
            cli,
            target,
            policy,
            shared: Arc::clone(&shared),
            on_update: Box::new(on_update),
            presented: PresentedSession::connecting(),
        };
        thread::Builder::new()
            .name("herdr-live-session".to_owned())
            .spawn(move || worker.run())
            .expect("spawn Herdr live session thread");
        Self { shared }
    }

    /// Retries now rather than after the current backoff, and starts the count of failed attempts
    /// afresh.
    pub fn reconnect(&self) {
        self.shared.control.lock().reconnect_requested = true;
        self.shared.changed.notify_all();
    }
}

impl Drop for LiveSession {
    fn drop(&mut self) {
        let mut control = self.shared.control.lock();
        control.stopped = true;
        if let Some(stream) = control.stream_shutdown.take() {
            stream.shutdown();
        }
        drop(control);
        self.shared.changed.notify_all();
    }
}

#[derive(Default)]
struct Shared {
    control: Mutex<Control>,
    changed: Condvar,
}

#[derive(Default)]
struct Control {
    stopped: bool,
    reconnect_requested: bool,
    /// Tells the current event stream's reader apart from those of earlier connections, which may
    /// still be winding down.
    stream_generation: u64,
    /// An event arrived after the last time this was cleared.
    invalidated: bool,
    stream_end: Option<StreamEnd>,
    stream_shutdown: Option<StreamShutdown>,
}

enum Wake {
    Stopped,
    Invalidated,
    StreamEnded(StreamEnd),
}

enum Pause {
    Stopped,
    ReconnectRequested,
    Elapsed,
}

impl Shared {
    /// Registers a new event stream and returns its generation, or `None` after a stop.
    fn begin_stream(&self, shutdown: StreamShutdown) -> Option<u64> {
        let mut control = self.control.lock();
        if control.stopped {
            shutdown.shutdown();
            return None;
        }
        control.stream_generation += 1;
        control.invalidated = false;
        control.stream_end = None;
        control.stream_shutdown = Some(shutdown);
        Some(control.stream_generation)
    }

    fn end_stream(&self) {
        if let Some(shutdown) = self.control.lock().stream_shutdown.take() {
            shutdown.shutdown();
        }
    }

    fn invalidate(&self, generation: u64) {
        let mut control = self.control.lock();
        if control.stream_generation == generation {
            control.invalidated = true;
            drop(control);
            self.changed.notify_all();
        }
    }

    fn end_of_stream(&self, generation: u64, end: StreamEnd) {
        let mut control = self.control.lock();
        if control.stream_generation == generation {
            control.stream_end = Some(end);
            drop(control);
            self.changed.notify_all();
        }
    }

    /// Forgets events seen so far. Called before a snapshot read so that an event arriving while
    /// the read is in flight is not forgotten with them.
    fn clear_invalidation(&self) {
        self.control.lock().invalidated = false;
    }

    fn wait_for_signal(&self) -> Wake {
        let mut control = self.control.lock();
        loop {
            if control.stopped {
                return Wake::Stopped;
            }
            if let Some(end) = control.stream_end.take() {
                return Wake::StreamEnded(end);
            }
            if control.invalidated {
                return Wake::Invalidated;
            }
            self.changed.wait(&mut control);
        }
    }

    fn pause(&self, timeout: Duration) -> Pause {
        let deadline = Instant::now() + timeout;
        let mut control = self.control.lock();
        loop {
            if control.stopped {
                return Pause::Stopped;
            }
            if mem::take(&mut control.reconnect_requested) {
                return Pause::ReconnectRequested;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Pause::Elapsed;
            }
            self.changed.wait_for(&mut control, remaining);
        }
    }
}

/// How one connection ended.
struct ConnectionEnd {
    outcome: Outcome,
    /// Whether a snapshot was read over it, which is what makes its end a loss rather than
    /// another failure to connect.
    established: bool,
}

enum Outcome {
    Stopped,
    Refused(Refusal),
    EventsLost,
    Failed(ConnectionFailure),
}

struct Worker {
    cli: HerdrCli,
    target: SessionTarget,
    policy: ReconnectPolicy,
    shared: Arc<Shared>,
    on_update: Box<dyn FnMut(PresentedSession) + Send>,
    /// What `on_update` was last given.
    presented: PresentedSession,
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
                // Herdr is healthy and only the cache is stale, so there is nothing to wait for.
                Outcome::EventsLost if end.established => continue,
                Outcome::Refused(refusal) => {
                    self.publish(PresentedSession::refused(refusal));
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

    fn serve_connection(&mut self) -> ConnectionEnd {
        // A request to retry made while no wait was under way has nothing left to cut short.
        self.shared.control.lock().reconnect_requested = false;
        if let Err(refusal) = compat::check(&self.cli, &self.target, STATUS_TIMEOUT) {
            return ConnectionEnd::unestablished(Outcome::Refused(refusal));
        }
        let (events, shutdown) = match EventStream::subscribe(self.target.socket_path()) {
            Ok(subscription) => subscription,
            Err(StreamEnd::EventsLost) => return ConnectionEnd::unestablished(Outcome::EventsLost),
            Err(StreamEnd::Failed(failure)) => {
                return ConnectionEnd::unestablished(Outcome::Failed(failure));
            }
        };
        let Some(generation) = self.shared.begin_stream(shutdown) else {
            return ConnectionEnd::unestablished(Outcome::Stopped);
        };
        spawn_reader(Arc::clone(&self.shared), generation, events);

        let end = self.follow_stream();
        self.shared.end_stream();
        end
    }

    /// Reads a snapshot, then again whenever an event arrives, until the stream ends.
    fn follow_stream(&mut self) -> ConnectionEnd {
        let mut established = false;
        loop {
            self.shared.clear_invalidation();
            match fetch_snapshot(&self.target) {
                Ok(snapshot) => {
                    established = true;
                    self.publish(PresentedSession::from_snapshot(snapshot));
                }
                Err(failure) => {
                    return ConnectionEnd {
                        outcome: Outcome::Failed(failure),
                        established,
                    };
                }
            }

            let outcome = match self.shared.wait_for_signal() {
                Wake::Invalidated => continue,
                Wake::Stopped => Outcome::Stopped,
                Wake::StreamEnded(StreamEnd::EventsLost) => Outcome::EventsLost,
                Wake::StreamEnded(StreamEnd::Failed(failure)) => Outcome::Failed(failure),
            };
            return ConnectionEnd {
                outcome,
                established,
            };
        }
    }

    fn publish_status(&mut self, status: ConnectionStatus) {
        let next = self.presented.clone().with_connection(status);
        self.publish(next);
    }

    fn publish(&mut self, next: PresentedSession) {
        if next == self.presented || self.shared.control.lock().stopped {
            return;
        }
        self.presented = next.clone();
        (self.on_update)(next);
    }
}

impl ConnectionEnd {
    fn unestablished(outcome: Outcome) -> Self {
        Self {
            outcome,
            established: false,
        }
    }
}

fn spawn_reader(shared: Arc<Shared>, generation: u64, mut events: EventStream) {
    thread::Builder::new()
        .name("herdr-event-stream".to_owned())
        .spawn(move || {
            loop {
                match events.wait_for_event() {
                    Ok(()) => shared.invalidate(generation),
                    Err(end) => {
                        shared.end_of_stream(generation, end);
                        return;
                    }
                }
            }
        })
        .expect("spawn Herdr event stream thread");
}

#[cfg(test)]
#[path = "live_tests.rs"]
mod tests;
