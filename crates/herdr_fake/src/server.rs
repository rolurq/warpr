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

use crate::{AgentState, SessionScript, SplitDirection};

/// How far `pane.resize` moves a divider when it is given no amount, as a share of the split.
const RESIZE_STEP: f32 = 0.05;

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
    /// The error the next creation request is answered with, if one is armed.
    creation_rejection: Mutex<Option<(String, String)>>,
    /// The error the next `pane.close` or `tab.close` request is answered with, if one is armed.
    close_rejection: Mutex<Option<(String, String)>>,
    /// The error the next `tab.rename` request is answered with, if one is armed.
    rename_rejection: Mutex<Option<(String, String)>>,
    /// The error the next `pane.split`, `pane.zoom` or `pane.resize` request is answered with, if
    /// one is armed.
    layout_rejection: Mutex<Option<(String, String)>>,
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

    fn emit(&self, name: &str, data: &Value) -> usize {
        let line = format!("{}\n", json!({ "event": name, "data": data }));
        let mut subscribers = self.subscribers.lock();
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

    /// Serves `workspace.create` and `tab.create`: changes the script as Herdr would, announces
    /// the result to subscribers and answers with it.
    fn create(&self, id: &Value, method: &str, params: &Value) -> Value {
        if let Some((code, message)) = self.creation_rejection.lock().take() {
            return error(id, code.as_str(), &message);
        }
        let label = params["label"].as_str();
        let focus = params["focus"].as_bool().unwrap_or(false);
        let mut script = self.script.lock();
        let (event, workspace_id, tab_id) = if method == "workspace.create" {
            let workspace_id = script.create_workspace(label, focus);
            let tab_id = format!("{workspace_id}:t1");
            ("workspace_created", workspace_id, tab_id)
        } else {
            let Some(workspace_id) = params["workspace_id"].as_str() else {
                return error(
                    id,
                    "invalid_request",
                    "invalid request: missing field `workspace_id`",
                );
            };
            match script.create_tab(workspace_id, label, focus) {
                Some(tab_id) => ("tab_created", workspace_id.to_owned(), tab_id),
                None => {
                    return error(
                        id,
                        "workspace_not_found",
                        &format!("workspace {workspace_id} not found"),
                    );
                }
            }
        };
        let snapshot = script.snapshot();
        drop(script);

        let mut result = json!({
            "type": event,
            "tab": find_entry(&snapshot, "tabs", "tab_id", &tab_id),
            "root_pane": find_entry(&snapshot, "panes", "tab_id", &tab_id),
        });
        if event == "workspace_created" {
            result["workspace"] =
                find_entry(&snapshot, "workspaces", "workspace_id", &workspace_id);
        }
        self.emit(event, &result);
        ok(id, result)
    }

    /// Serves `pane.close`: closes the pane as Herdr would, announces `pane_closed` to
    /// subscribers and answers `ok`.
    fn close_pane(&self, id: &Value, params: &Value) -> Value {
        let Some(pane_id) = params["pane_id"].as_str() else {
            return error(
                id,
                "invalid_request",
                "invalid request: missing field `pane_id`",
            );
        };
        if let Some((code, message)) = self.close_rejection.lock().take() {
            return error(id, code.as_str(), &message);
        }
        let closed = self.script.lock().close_pane(pane_id);
        let Some(workspace_id) = closed else {
            return error(id, "pane_not_found", &format!("pane {pane_id} not found"));
        };
        self.emit(
            "pane_closed",
            &json!({ "type": "pane_closed", "pane_id": pane_id, "workspace_id": workspace_id }),
        );
        ok(id, json!({ "type": "ok" }))
    }

    /// Serves `tab.close`: closes the Herdr tab as Herdr would, announces `tab_closed` to
    /// subscribers and answers `ok`.
    fn close_tab(&self, id: &Value, params: &Value) -> Value {
        let Some(tab_id) = params["tab_id"].as_str() else {
            return error(
                id,
                "invalid_request",
                "invalid request: missing field `tab_id`",
            );
        };
        if let Some((code, message)) = self.close_rejection.lock().take() {
            return error(id, code.as_str(), &message);
        }
        let closed = self.script.lock().close_tab(tab_id);
        let Some(workspace_id) = closed else {
            return error(id, "tab_not_found", &format!("tab {tab_id} not found"));
        };
        self.emit(
            "tab_closed",
            &json!({ "type": "tab_closed", "tab_id": tab_id, "workspace_id": workspace_id }),
        );
        ok(id, json!({ "type": "ok" }))
    }

    /// Serves `tab.rename`: relabels the Herdr tab as Herdr would, announces `tab_renamed` to
    /// subscribers and answers with the tab.
    fn rename_tab(&self, id: &Value, params: &Value) -> Value {
        let (Some(tab_id), Some(label)) = (params["tab_id"].as_str(), params["label"].as_str())
        else {
            return error(
                id,
                "invalid_request",
                "invalid request: missing field `tab_id` or `label`",
            );
        };
        if let Some((code, message)) = self.rename_rejection.lock().take() {
            return error(id, code.as_str(), &message);
        }
        let mut script = self.script.lock();
        if script.rename_tab(tab_id, label).is_none() {
            return error(id, "tab_not_found", &format!("tab {tab_id} not found"));
        }
        let tab = script.snapshot()["tabs"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|entry| entry["tab_id"] == tab_id)
            .cloned()
            .unwrap_or(Value::Null);
        drop(script);
        let result = json!({ "type": "tab_renamed", "tab": tab });
        self.emit("tab_renamed", &result);
        ok(id, json!({ "type": "tab_info", "tab": tab }))
    }

    /// Serves `pane.split`: splits the pane as Herdr does without `focus`, announces the new pane
    /// and the changed layout to subscribers and answers with the new pane.
    fn split_pane(&self, id: &Value, params: &Value) -> Value {
        let Some(pane_id) = params["target_pane_id"].as_str() else {
            return error(
                id,
                "invalid_request",
                "invalid request: missing field `target_pane_id`",
            );
        };
        let direction = match params["direction"].as_str() {
            Some("right") => SplitDirection::Right,
            Some("down") => SplitDirection::Down,
            other => {
                return error(
                    id,
                    "invalid_request",
                    &format!(
                        "invalid request: unknown variant {other:?}, expected `right` or `down`"
                    ),
                );
            }
        };
        if let Some((code, message)) = self.layout_rejection.lock().take() {
            return error(id, code.as_str(), &message);
        }
        let mut script = self.script.lock();
        let Some(new_pane_id) = script.split_pane(pane_id, direction) else {
            return error(id, "pane_not_found", &format!("pane {pane_id} not found"));
        };
        let snapshot = script.snapshot();
        drop(script);

        let pane = find_entry(&snapshot, "panes", "pane_id", &new_pane_id);
        self.emit(
            "pane_created",
            &json!({ "type": "pane_created", "pane": pane }),
        );
        self.emit_layout_updated(&snapshot, &new_pane_id);
        ok(id, json!({ "type": "pane_info", "pane": pane }))
    }

    /// Serves `pane.zoom`: zooms the pane or unzooms its tab as Herdr does, announces a moved
    /// focus and the layout to subscribers and answers with the outcome.
    fn zoom_pane(&self, id: &Value, params: &Value) -> Value {
        let Some(pane_id) = params["pane_id"].as_str() else {
            return error(
                id,
                "invalid_request",
                "invalid request: missing field `pane_id`",
            );
        };
        let zoomed = match params["mode"].as_str() {
            None | Some("toggle") => None,
            Some("on") => Some(true),
            Some("off") => Some(false),
            Some(other) => {
                return error(
                    id,
                    "invalid_request",
                    &format!("invalid request: unknown variant `{other}`"),
                );
            }
        };
        if let Some((code, message)) = self.layout_rejection.lock().take() {
            return error(id, code.as_str(), &message);
        }
        let mut script = self.script.lock();
        let Some(change) = script.zoom_pane(pane_id, zoomed) else {
            return error(id, "pane_not_found", &format!("pane {pane_id} not found"));
        };
        let snapshot = script.snapshot();
        drop(script);

        if change.focus_changed {
            let workspace_id =
                find_entry(&snapshot, "panes", "pane_id", pane_id)["workspace_id"].clone();
            self.emit(
                "pane_focused",
                &json!({ "type": "pane_focused", "pane_id": pane_id, "workspace_id": workspace_id }),
            );
        }
        let layout = self.emit_layout_updated(&snapshot, pane_id);
        ok(
            id,
            json!({
                "type": "pane_zoom",
                "zoom": {
                    "changed": change.zoom_changed || change.focus_changed,
                    "zoom_changed": change.zoom_changed,
                    "focus_changed": change.focus_changed,
                    "pane_id": pane_id,
                    "focused_pane_id": layout["focused_pane_id"],
                    "zoomed": change.zoomed,
                    "layout": layout,
                },
            }),
        )
    }

    /// Serves `pane.resize`: moves the divider by Herdr's default step, announces the layout to
    /// subscribers and answers with the outcome.
    fn resize_pane(&self, id: &Value, params: &Value) -> Value {
        let Some(pane_id) = params["pane_id"].as_str() else {
            return error(
                id,
                "invalid_request",
                "invalid request: missing field `pane_id`",
            );
        };
        let (direction, step) = match params["direction"].as_str() {
            Some("left") => (SplitDirection::Right, -RESIZE_STEP),
            Some("right") => (SplitDirection::Right, RESIZE_STEP),
            Some("up") => (SplitDirection::Down, -RESIZE_STEP),
            Some("down") => (SplitDirection::Down, RESIZE_STEP),
            other => {
                return error(
                    id,
                    "invalid_request",
                    &format!("invalid request: unknown variant {other:?}"),
                );
            }
        };
        if let Some((code, message)) = self.layout_rejection.lock().take() {
            return error(id, code.as_str(), &message);
        }
        let mut script = self.script.lock();
        let Some(changed) = script.move_divider(pane_id, direction, step) else {
            return error(id, "pane_not_found", &format!("pane {pane_id} not found"));
        };
        let snapshot = script.snapshot();
        drop(script);

        let layout = self.emit_layout_updated(&snapshot, pane_id);
        let mut resize = json!({
            "changed": changed,
            "pane_id": pane_id,
            "focused_pane_id": layout["focused_pane_id"],
            "layout": layout,
        });
        if !changed {
            resize["reason"] = json!("unchanged");
        }
        ok(id, json!({ "type": "pane_resize", "resize": resize }))
    }

    /// Announces `layout_updated` for the Herdr tab holding `pane_id` and returns its layout as
    /// `snapshot` reports it.
    fn emit_layout_updated(&self, snapshot: &Value, pane_id: &str) -> Value {
        let tab_id = &find_entry(snapshot, "panes", "pane_id", pane_id)["tab_id"];
        let layout = find_entry(
            snapshot,
            "layouts",
            "tab_id",
            tab_id.as_str().unwrap_or_default(),
        );
        self.emit(
            "layout_updated",
            &json!({ "type": "layout_updated", "layout": layout }),
        );
        layout
    }

    /// Serves `worktree.create`: adds the linked worktree as a workspace, announces it to
    /// subscribers and answers with it. Like Herdr, it refuses a directory that is not in a
    /// repository.
    fn create_worktree(&self, id: &Value, params: &Value) -> Value {
        if let Some((code, message)) = self.creation_rejection.lock().take() {
            return error(id, code.as_str(), &message);
        }
        let (Some(cwd), Some(branch)) = (params["cwd"].as_str(), params["branch"].as_str()) else {
            return error(
                id,
                "invalid_request",
                "invalid request: missing field `cwd` or `branch`",
            );
        };
        let label = params["label"].as_str().unwrap_or(branch);
        let focus = params["focus"].as_bool().unwrap_or(false);
        let mut script = self.script.lock();
        let Some(workspace_id) = script.create_worktree(cwd, label, focus) else {
            return error(
                id,
                "worktree_open_failed",
                "Herdr worktree actions require a path inside a Git work tree",
            );
        };
        let workspace = script.snapshot()["workspaces"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|entry| entry["workspace_id"] == workspace_id.as_str())
            .cloned()
            .unwrap_or(Value::Null);
        drop(script);

        let result = json!({
            "type": "worktree_created",
            "worktree": workspace["worktree"],
            "workspace": workspace,
        });
        self.emit("worktree_created", &result);
        ok(id, result)
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
                creation_rejection: Mutex::new(None),
                close_rejection: Mutex::new(None),
                rename_rejection: Mutex::new(None),
                layout_rejection: Mutex::new(None),
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
        self.shared.emit(name, &data)
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

    /// Answers the next `workspace.create`, `tab.create` or `worktree.create` request with the
    /// error `code` and `message` instead of creating anything, as Herdr does for a request it
    /// refuses.
    pub fn reject_next_creation(&self, code: &str, message: &str) {
        *self.shared.creation_rejection.lock() = Some((code.to_owned(), message.to_owned()));
    }

    /// Answers the next `pane.close` or `tab.close` request with the error `code` and `message`
    /// instead of closing anything, as Herdr does for a request it refuses.
    pub fn reject_next_close(&self, code: &str, message: &str) {
        *self.shared.close_rejection.lock() = Some((code.to_owned(), message.to_owned()));
    }

    /// Answers the next `tab.rename` request with the error `code` and `message` instead of
    /// renaming anything, as Herdr does for a request it refuses.
    pub fn reject_next_rename(&self, code: &str, message: &str) {
        *self.shared.rename_rejection.lock() = Some((code.to_owned(), message.to_owned()));
    }

    /// Answers the next `pane.split`, `pane.zoom` or `pane.resize` request with the error `code`
    /// and `message` instead of changing anything, as Herdr does for a request it refuses.
    pub fn reject_next_layout_change(&self, code: &str, message: &str) {
        *self.shared.layout_rejection.lock() = Some((code.to_owned(), message.to_owned()));
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
            "workspace.create" | "tab.create" => shared.create(&id, method, &request["params"]),
            "worktree.create" => shared.create_worktree(&id, &request["params"]),
            "pane.close" => shared.close_pane(&id, &request["params"]),
            "tab.close" => shared.close_tab(&id, &request["params"]),
            "tab.rename" => shared.rename_tab(&id, &request["params"]),
            "pane.split" => shared.split_pane(&id, &request["params"]),
            "pane.zoom" => shared.zoom_pane(&id, &request["params"]),
            "pane.resize" => shared.resize_pane(&id, &request["params"]),
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

/// The entry of the `list` array of a snapshot whose `key` is `wanted`, or null.
fn find_entry(snapshot: &Value, list: &str, key: &str, wanted: &str) -> Value {
    snapshot[list]
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| entry[key] == wanted)
        .cloned()
        .unwrap_or(Value::Null)
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
