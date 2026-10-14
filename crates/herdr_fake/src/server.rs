use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use parking_lot::{Condvar, Mutex};
use serde_json::{Value, json};

use crate::{AgentState, SessionScript};

/// Subscription types Herdr accepts without naming a pane.
const LIFECYCLE_SUBSCRIPTIONS: [&str; 24] = [
    "workspace.created",
    "workspace.updated",
    "workspace.metadata_updated",
    "workspace.renamed",
    "workspace.moved",
    "workspace.reordered",
    "workspace.closed",
    "workspace.focused",
    "worktree.created",
    "worktree.opened",
    "worktree.removed",
    "tab.created",
    "tab.closed",
    "tab.focused",
    "tab.renamed",
    "tab.moved",
    "pane.created",
    "pane.closed",
    "pane.updated",
    "pane.focused",
    "pane.moved",
    "pane.exited",
    "pane.agent_detected",
    "layout.updated",
];

/// Subscription types Herdr only accepts for one named pane.
const PANE_SUBSCRIPTIONS: [&str; 3] = [
    "pane.output_matched",
    "pane.agent_status_changed",
    "pane.scroll_changed",
];

const AGENT_STATUS_CHANGED: &str = "pane.agent_status_changed";

/// How a pane-scoped subscription is remembered among a subscriber's event names.
fn pane_event_key(kind: &str, pane_id: &str) -> String {
    format!("{kind}@{pane_id}")
}

const EVENTS_LOST_MESSAGE: &str =
    "event subscription fell behind retained history; resubscribe and resync with session.snapshot";

/// How long a test waits for something the fake is expected to do before giving up.
const WAIT_LIMIT: Duration = Duration::from_secs(10);

struct Subscriber {
    connection_id: u64,
    stream: UnixStream,
    /// The `id` of the `events.subscribe` request, which Herdr repeats in an `events_lost` error.
    request_id: Value,
    /// Event names (`workspace_renamed`) rather than subscription types (`workspace.renamed`).
    events: HashSet<String>,
}

#[derive(Default)]
struct SnapshotGate {
    state: Mutex<GateState>,
    changed: Condvar,
}

#[derive(Default)]
struct GateState {
    held: bool,
    intercepted: usize,
}

impl SnapshotGate {
    /// Blocks while the gate is held, so a response computed earlier is delivered later.
    fn pass(&self) {
        let mut state = self.state.lock();
        if !state.held {
            return;
        }
        state.intercepted += 1;
        self.changed.notify_all();
        while state.held {
            self.changed.wait_for(&mut state, WAIT_LIMIT);
        }
        state.intercepted -= 1;
    }

    fn set_held(&self, held: bool) {
        self.state.lock().held = held;
        self.changed.notify_all();
    }
}

struct Shared {
    script: Mutex<SessionScript>,
    connections: Mutex<HashMap<u64, UnixStream>>,
    subscribers: Mutex<Vec<Subscriber>>,
    snapshot_gate: SnapshotGate,
    next_connection_id: AtomicU64,
    requests_served: AtomicUsize,
    methods_served: Mutex<Vec<String>>,
    /// How many of the next `events.subscribe` requests are answered with `events_lost`.
    subscriptions_to_lose: Mutex<usize>,
}

impl Shared {
    fn close_connection(&self, connection_id: u64) {
        self.connections.lock().remove(&connection_id);
        self.subscribers
            .lock()
            .retain(|subscriber| subscriber.connection_id != connection_id);
    }

    fn close_all_connections(&self) {
        self.subscribers.lock().clear();
        for (_, stream) in self.connections.lock().drain() {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }

    fn lose_next_subscription(&self) -> bool {
        let mut remaining = self.subscriptions_to_lose.lock();
        let lose = *remaining > 0;
        *remaining = remaining.saturating_sub(1);
        lose
    }

    /// Validates `events.subscribe` params as Herdr does: every entry must be a known type, pane
    /// scoped types must name a pane, and that pane must exist.
    fn parse_subscriptions(
        &self,
        params: &Value,
    ) -> Result<HashSet<String>, (&'static str, String)> {
        let invalid = |message: String| ("invalid_request", format!("invalid request: {message}"));
        let entries = params["subscriptions"]
            .as_array()
            .ok_or_else(|| invalid("missing field `subscriptions`".to_owned()))?;

        let known_panes: HashSet<String> = self.script.lock().snapshot()["panes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|pane| pane["pane_id"].as_str().map(str::to_owned))
            .collect();

        let mut events = HashSet::new();
        for entry in entries {
            let kind = entry["type"]
                .as_str()
                .ok_or_else(|| invalid("missing field `type`".to_owned()))?;
            if PANE_SUBSCRIPTIONS.contains(&kind) {
                let pane = entry["pane_id"]
                    .as_str()
                    .ok_or_else(|| invalid("missing field `pane_id`".to_owned()))?;
                if !known_panes.contains(pane) {
                    return Err(("pane_not_found", format!("pane {pane} not found")));
                }
            } else if !LIFECYCLE_SUBSCRIPTIONS.contains(&kind) {
                return Err(invalid(format!("unknown variant `{kind}`")));
            }
            events.insert(
                match entry["pane_id"]
                    .as_str()
                    .filter(|_| PANE_SUBSCRIPTIONS.contains(&kind))
                {
                    Some(pane) => pane_event_key(kind, pane),
                    None => kind.replace('.', "_"),
                },
            );
        }
        Ok(events)
    }
}

struct Listener {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

/// A running fake Herdr session server. Stops and removes its socket when dropped.
pub struct FakeSession {
    socket_path: PathBuf,
    shared: Arc<Shared>,
    listener: Mutex<Option<Listener>>,
}

impl FakeSession {
    pub(crate) fn start(socket_path: PathBuf, script: SessionScript) -> Self {
        let session = Self {
            socket_path,
            shared: Arc::new(Shared {
                script: Mutex::new(script),
                connections: Mutex::new(HashMap::new()),
                subscribers: Mutex::new(Vec::new()),
                snapshot_gate: SnapshotGate::default(),
                next_connection_id: AtomicU64::new(0),
                requests_served: AtomicUsize::new(0),
                methods_served: Mutex::new(Vec::new()),
                subscriptions_to_lose: Mutex::new(0),
            }),
            listener: Mutex::new(None),
        };
        session.go_online();
        session
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// How many requests this session has answered, across all connections.
    pub fn requests_served(&self) -> usize {
        self.shared.requests_served.load(Ordering::SeqCst)
    }

    /// The method of every request this session has answered, in the order they arrived.
    pub fn served_methods(&self) -> Vec<String> {
        self.shared.methods_served.lock().clone()
    }

    /// Replaces what the session reports from now on, as another Herdr client changing the
    /// session would. Nothing is emitted: pair it with [`Self::emit_event`] to announce the
    /// change, or leave it unannounced to model events that were lost.
    pub fn set_script(&self, script: SessionScript) {
        *self.shared.script.lock() = script;
    }

    /// Pushes an event to every connection that subscribed to it, framed as Herdr frames it:
    /// `{"event": <name>, "data": <data>}`. `name` is the event name (`workspace_renamed`), not
    /// the subscription type (`workspace.renamed`). Returns how many connections received it.
    pub fn emit_event(&self, name: &str, data: Value) -> usize {
        let line = format!("{}\n", json!({ "event": name, "data": data }));
        let mut subscribers = self.shared.subscribers.lock();
        let mut delivered = 0;
        subscribers.retain_mut(|subscriber| {
            if !subscriber.events.contains(name) {
                return true;
            }
            let written = subscriber.stream.write_all(line.as_bytes()).is_ok();
            delivered += usize::from(written);
            written
        });
        delivered
    }

    /// Pushes `pane.agent_status_changed` for `pane_id` to the connections that subscribed to
    /// that pane, framed as Herdr frames pane-scoped events: named with a dot, unlike lifecycle
    /// events. Returns how many connections received it.
    pub fn emit_agent_status_changed(
        &self,
        pane_id: &str,
        agent: &str,
        state: AgentState,
    ) -> usize {
        let workspace_id = pane_id.split(':').next().unwrap_or(pane_id);
        let line = format!(
            "{}\n",
            json!({
                "event": AGENT_STATUS_CHANGED,
                "data": {
                    "pane_id": pane_id,
                    "workspace_id": workspace_id,
                    "agent": agent,
                    "agent_status": state.wire_name(),
                },
            })
        );
        let key = pane_event_key(AGENT_STATUS_CHANGED, pane_id);
        let mut subscribers = self.shared.subscribers.lock();
        let mut delivered = 0;
        subscribers.retain_mut(|subscriber| {
            if !subscriber.events.contains(&key) {
                return true;
            }
            let written = subscriber.stream.write_all(line.as_bytes()).is_ok();
            delivered += usize::from(written);
            written
        });
        delivered
    }

    /// The panes whose agent status the open subscriptions follow, one entry per subscription.
    pub fn agent_status_subscriptions(&self) -> Vec<String> {
        let prefix = pane_event_key(AGENT_STATUS_CHANGED, "");
        let mut panes: Vec<_> = self
            .shared
            .subscribers
            .lock()
            .iter()
            .flat_map(|subscriber| {
                subscriber
                    .events
                    .iter()
                    .filter_map(|event| event.strip_prefix(&prefix).map(str::to_owned))
            })
            .collect();
        panes.sort();
        panes
    }

    /// Ends every subscription the way Herdr does when subscribers fall behind its retained
    /// history: an `events_lost` error carrying the request's `id`, then the connection closes.
    pub fn lose_events(&self) {
        for subscriber in self.shared.subscribers.lock().drain(..) {
            let _ = write_line(
                &subscriber.stream,
                &error(&subscriber.request_id, "events_lost", EVENTS_LOST_MESSAGE),
            );
            let _ = subscriber.stream.shutdown(Shutdown::Both);
        }
    }

    /// Answers the next `times` subscription requests with `events_lost` instead of starting
    /// them, as Herdr does when the history is overrun while a subscription is being set up.
    pub fn lose_events_on_next_subscriptions(&self, times: usize) {
        *self.shared.subscriptions_to_lose.lock() = times;
    }

    /// How many connections are subscribed to events.
    pub fn subscriber_count(&self) -> usize {
        self.shared.subscribers.lock().len()
    }

    /// Closes every open connection without a word while the socket keeps listening.
    pub fn drop_connections(&self) {
        self.shared.close_all_connections();
    }

    /// Stops the server as if Herdr had exited: connections close and the socket disappears, so
    /// connecting fails until [`Self::go_online`].
    pub fn go_offline(&self) {
        let listener = self.listener.lock().take();
        if let Some(listener) = listener {
            listener.stop.store(true, Ordering::SeqCst);
            // Unblocks the accept loop so it can observe `stop`.
            let _ = UnixStream::connect(&self.socket_path);
            let _ = listener.thread.join();
        }
        self.shared.close_all_connections();
        let _ = std::fs::remove_file(&self.socket_path);
    }

    /// Starts listening again, reporting whatever the script says now. Subscriptions from before
    /// the outage do not survive it.
    pub fn go_online(&self) {
        let mut listener = self.listener.lock();
        if listener.is_some() {
            return;
        }
        let socket = UnixListener::bind(&self.socket_path)
            .unwrap_or_else(|error| panic!("bind {}: {error}", self.socket_path.display()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = thread::spawn({
            let shared = Arc::clone(&self.shared);
            let stop = Arc::clone(&stop);
            move || accept_connections(&socket, &shared, &stop)
        });
        *listener = Some(Listener { stop, thread });
    }

    /// Holds back every `session.snapshot` and `agent.list` response until the returned guard is
    /// dropped. A held response is computed when the request arrives, so it goes stale if the
    /// script changes before it is released, as a response overtaken by an event would.
    pub fn pause_snapshots(&self) -> SnapshotPause {
        self.shared.snapshot_gate.set_held(true);
        SnapshotPause {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl Drop for FakeSession {
    fn drop(&mut self) {
        self.shared.snapshot_gate.set_held(false);
        self.go_offline();
    }
}

/// Holds back snapshot responses of a [`FakeSession`] while it lives.
pub struct SnapshotPause {
    shared: Arc<Shared>,
}

impl SnapshotPause {
    /// Blocks until a `session.snapshot` request is being held. Panics if none arrives.
    pub fn wait_until_requested(&self) {
        let gate = &self.shared.snapshot_gate;
        let mut state = gate.state.lock();
        while state.intercepted == 0 {
            let waited = gate.changed.wait_for(&mut state, WAIT_LIMIT);
            assert!(
                !waited.timed_out(),
                "no session.snapshot request arrived while paused"
            );
        }
    }
}

impl Drop for SnapshotPause {
    fn drop(&mut self) {
        self.shared.snapshot_gate.set_held(false);
    }
}

fn accept_connections(listener: &UnixListener, shared: &Arc<Shared>, stop: &AtomicBool) {
    for stream in listener.incoming() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let Ok(stream) = stream else { continue };
        let Ok(registered) = stream.try_clone() else {
            continue;
        };
        let connection_id = shared.next_connection_id.fetch_add(1, Ordering::SeqCst);
        shared.connections.lock().insert(connection_id, registered);
        let shared = Arc::clone(shared);
        thread::spawn(move || {
            serve_connection(stream, &shared, connection_id);
            shared.close_connection(connection_id);
        });
    }
}

fn serve_connection(stream: UnixStream, shared: &Shared, connection_id: u64) {
    let Ok(reader_stream) = stream.try_clone() else {
        return;
    };
    let writer = stream;
    for line in BufReader::new(reader_stream).lines() {
        let Ok(line) = line else { return };
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            return;
        };
        let id = request["id"].clone();
        let method = request["method"].as_str().unwrap_or_default();
        shared.methods_served.lock().push(method.to_owned());
        shared.requests_served.fetch_add(1, Ordering::SeqCst);
        let response = match method {
            "ping" => ok(
                &id,
                json!({
                    "type": "pong",
                    "version": "0.9.3",
                    "protocol": 22,
                    "capabilities": {},
                }),
            ),
            "session.snapshot" => {
                let response = ok(
                    &id,
                    json!({ "type": "session_snapshot", "snapshot": shared.script.lock().snapshot() }),
                );
                shared.snapshot_gate.pass();
                response
            }
            "pane.list" => {
                let panes = shared.script.lock().snapshot()["panes"].clone();
                ok(&id, json!({ "type": "pane_list", "panes": panes }))
            }
            "agent.list" => {
                let agents = shared.script.lock().snapshot()["agents"].clone();
                let response = ok(&id, json!({ "type": "agent_list", "agents": agents }));
                shared.snapshot_gate.pass();
                response
            }
            "events.subscribe" => {
                if shared.lose_next_subscription() {
                    let _ = write_line(&writer, &error(&id, "events_lost", EVENTS_LOST_MESSAGE));
                    return;
                }
                let events = match shared.parse_subscriptions(&request["params"]) {
                    Ok(events) => events,
                    Err((code, message)) => {
                        let _ = write_line(&writer, &error(&id, code, &message));
                        return;
                    }
                };
                // Registering under the lock keeps an emitted event from overtaking the response.
                let mut subscribers = shared.subscribers.lock();
                if write_line(&writer, &ok(&id, json!({ "type": "subscription_started" }))).is_err()
                {
                    return;
                }
                match writer.try_clone() {
                    Ok(stream) => subscribers.push(Subscriber {
                        connection_id,
                        stream,
                        request_id: id,
                        events,
                    }),
                    Err(_) => return,
                }
                continue;
            }
            other => error(
                &id,
                "invalid_request",
                &format!("invalid request: unknown variant `{other}`"),
            ),
        };
        if write_line(&writer, &response).is_err() {
            return;
        }
    }
}

fn write_line(mut stream: &UnixStream, value: &Value) -> std::io::Result<()> {
    writeln!(stream, "{value}")
}

fn ok(id: &Value, result: Value) -> Value {
    json!({ "id": id, "result": result })
}

fn error(id: &Value, code: &str, message: &str) -> Value {
    json!({ "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
