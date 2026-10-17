use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use serde_json::{Value, json};

use super::*;
use crate::{FakeHerdr, PaneTree, SplitDirection, TabScript};

struct Client {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Client {
    fn connect(session: &FakeSession) -> Self {
        let stream = UnixStream::connect(session.socket_path()).expect("connect to fake session");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set read timeout");
        let reader = BufReader::new(stream.try_clone().expect("clone stream"));
        Self { stream, reader }
    }

    fn send(&mut self, id: &str, method: &str, params: Value) {
        writeln!(
            self.stream,
            "{}",
            json!({ "id": id, "method": method, "params": params })
        )
        .expect("send request");
    }

    /// The next line as JSON, or `None` once the server has closed the connection.
    fn read(&mut self) -> Option<Value> {
        let mut line = String::new();
        let read = self.reader.read_line(&mut line).unwrap_or(0);
        (read > 0).then(|| serde_json::from_str(&line).expect("valid json line"))
    }

    fn subscribe(&mut self, id: &str, subscriptions: Value) -> Option<Value> {
        self.send(
            id,
            "events.subscribe",
            json!({ "subscriptions": subscriptions }),
        );
        self.read()
    }

    fn snapshot_labels(&mut self) -> Vec<String> {
        self.send("snap", "session.snapshot", json!({}));
        let response = self.read().expect("snapshot response");
        response["result"]["snapshot"]["workspaces"]
            .as_array()
            .expect("workspaces")
            .iter()
            .map(|workspace| workspace["label"].as_str().expect("label").to_owned())
            .collect()
    }
}

fn session_with(fake: &FakeHerdr, labels: &[&str]) -> FakeSession {
    let script = labels.iter().fold(SessionScript::new(), |script, label| {
        script.workspace(label)
    });
    fake.start_session("default", script)
}

#[test]
fn events_reach_only_connections_subscribed_to_that_event() {
    let fake = FakeHerdr::new();
    let session = session_with(&fake, &["api"]);
    let mut renames = Client::connect(&session);
    let mut closes = Client::connect(&session);
    renames.subscribe("a", json!([{ "type": "workspace.renamed" }]));
    closes.subscribe("b", json!([{ "type": "workspace.closed" }]));

    let delivered = session.emit_event("workspace_renamed", json!({ "label": "x" }));

    assert_eq!(delivered, 1);
    assert_eq!(
        renames.read(),
        Some(json!({ "event": "workspace_renamed", "data": { "label": "x" } }))
    );
    assert_eq!(session.emit_event("workspace_created", json!({})), 0);
}

#[test]
fn a_subscription_for_an_event_herdr_does_not_have_is_rejected_and_closed() {
    let fake = FakeHerdr::new();
    let session = session_with(&fake, &["api"]);
    let mut client = Client::connect(&session);

    let response = client
        .subscribe("s", json!([{ "type": "workspace.vaporised" }]))
        .expect("an error response");

    assert_eq!(response["id"], "s");
    assert_eq!(response["error"]["code"], "invalid_request");
    assert_eq!(client.read(), None);
    assert_eq!(session.subscriber_count(), 0);
}

#[test]
fn pane_scoped_subscriptions_must_name_a_pane_that_exists() {
    let fake = FakeHerdr::new();
    let session = session_with(&fake, &["api"]);

    let missing_pane = Client::connect(&session)
        .subscribe("a", json!([{ "type": "pane.agent_status_changed" }]))
        .expect("an error response");
    let unknown_pane = Client::connect(&session)
        .subscribe(
            "b",
            json!([{ "type": "pane.agent_status_changed", "pane_id": "w9:p9" }]),
        )
        .expect("an error response");
    let known_pane = Client::connect(&session)
        .subscribe(
            "c",
            json!([{ "type": "pane.agent_status_changed", "pane_id": "w1:p1" }]),
        )
        .expect("a response");

    assert_eq!(missing_pane["error"]["code"], "invalid_request");
    assert_eq!(unknown_pane["error"]["code"], "pane_not_found");
    assert_eq!(known_pane["result"]["type"], "subscription_started");
}

#[test]
fn lost_events_end_every_subscription_as_herdr_ends_them() {
    let fake = FakeHerdr::new();
    let session = session_with(&fake, &["api"]);
    let mut client = Client::connect(&session);
    client.subscribe("sub-1", json!([{ "type": "workspace.created" }]));

    session.lose_events();

    // Captured from Herdr 0.9.3 when a subscriber fell behind.
    assert_eq!(
        client.read(),
        Some(json!({
            "id": "sub-1",
            "error": {
                "code": "events_lost",
                "message": "event subscription fell behind retained history; resubscribe and resync with session.snapshot",
            },
        }))
    );
    assert_eq!(client.read(), None);
    assert_eq!(session.subscriber_count(), 0);
}

#[test]
fn the_next_subscriptions_can_be_answered_with_lost_events() {
    let fake = FakeHerdr::new();
    let session = session_with(&fake, &["api"]);
    session.lose_events_on_next_subscriptions(1);

    let first = Client::connect(&session)
        .subscribe("a", json!([{ "type": "workspace.created" }]))
        .expect("an error response");
    let second = Client::connect(&session)
        .subscribe("b", json!([{ "type": "workspace.created" }]))
        .expect("a response");

    assert_eq!(first["error"]["code"], "events_lost");
    assert_eq!(second["result"]["type"], "subscription_started");
}

#[test]
fn a_replaced_script_is_what_later_snapshots_report() {
    let fake = FakeHerdr::new();
    let session = session_with(&fake, &["api"]);
    let mut client = Client::connect(&session);
    assert_eq!(client.snapshot_labels(), ["api"]);

    session.set_script(SessionScript::new().workspace("web").workspace("docs"));

    assert_eq!(client.snapshot_labels(), ["web", "docs"]);
}

#[test]
fn dropped_connections_end_without_a_response_while_the_socket_keeps_listening() {
    let fake = FakeHerdr::new();
    let session = session_with(&fake, &["api"]);
    let mut dropped = Client::connect(&session);
    dropped.subscribe("a", json!([{ "type": "workspace.created" }]));

    session.drop_connections();

    assert_eq!(dropped.read(), None);
    assert_eq!(session.subscriber_count(), 0);
    assert_eq!(Client::connect(&session).snapshot_labels(), ["api"]);
}

#[test]
fn going_offline_closes_connections_and_removes_the_socket_until_back_online() {
    let fake = FakeHerdr::new();
    let session = session_with(&fake, &["api"]);
    let mut client = Client::connect(&session);
    client.subscribe("a", json!([{ "type": "workspace.created" }]));

    session.go_offline();

    assert_eq!(client.read(), None);
    assert!(UnixStream::connect(session.socket_path()).is_err());

    session.set_script(SessionScript::new().workspace("web"));
    session.go_online();

    assert_eq!(Client::connect(&session).snapshot_labels(), ["web"]);
    assert_eq!(
        session.subscriber_count(),
        0,
        "a subscription survived the outage"
    );
}

#[test]
fn a_paused_snapshot_is_computed_when_requested_and_delivered_when_released() {
    let fake = FakeHerdr::new();
    let session = session_with(&fake, &["api"]);
    let mut client = Client::connect(&session);
    let pause = session.pause_snapshots();

    client.send("snap", "session.snapshot", json!({}));
    pause.wait_until_requested();
    session.set_script(SessionScript::new().workspace("web"));
    drop(pause);

    let response = client.read().expect("snapshot response");
    let label = &response["result"]["snapshot"]["workspaces"][0]["label"];
    assert_eq!(label, "api");
    assert_eq!(client.snapshot_labels(), ["web"]);
}

#[test]
fn creating_a_workspace_announces_workspace_created_and_answers_with_it() {
    let fake = FakeHerdr::new();
    let session = session_with(&fake, &["api"]);
    let mut watcher = Client::connect(&session);
    watcher.subscribe(
        "w",
        json!([{ "type": "workspace.created" }, { "type": "tab.created" }]),
    );
    let mut client = Client::connect(&session);

    client.send(
        "c",
        "workspace.create",
        json!({ "label": "web", "focus": true }),
    );
    let response = client.read().expect("a response");

    assert_eq!(response["result"]["type"], "workspace_created");
    assert_eq!(response["result"]["workspace"]["label"], "web");
    assert_eq!(response["result"]["workspace"]["focused"], true);
    let event = watcher.read().expect("an event");
    assert_eq!(event["event"], "workspace_created");
    assert_eq!(event["data"], response["result"]);
    assert_eq!(client.snapshot_labels(), ["api", "web"]);
}

#[test]
fn creating_a_tab_announces_tab_created_and_leaves_other_workspaces_alone() {
    let fake = FakeHerdr::new();
    let session = session_with(&fake, &["api", "web"]);
    let mut watcher = Client::connect(&session);
    watcher.subscribe("w", json!([{ "type": "tab.created" }]));
    let mut client = Client::connect(&session);

    client.send("c", "tab.create", json!({ "workspace_id": "w2" }));
    let response = client.read().expect("a response");

    assert_eq!(response["result"]["type"], "tab_created");
    assert_eq!(response["result"]["tab"]["tab_id"], "w2:t2");
    assert_eq!(response["result"]["tab"]["workspace_id"], "w2");
    assert_eq!(watcher.read().expect("an event")["event"], "tab_created");
    client.send("s", "session.snapshot", json!({}));
    let snapshot = client.read().expect("a snapshot");
    let tab_counts: Vec<_> = snapshot["result"]["snapshot"]["workspaces"]
        .as_array()
        .expect("workspaces")
        .iter()
        .map(|workspace| workspace["tab_count"].clone())
        .collect();
    assert_eq!(tab_counts, [json!(1), json!(2)]);
}

#[test]
fn a_rejected_creation_changes_nothing_and_announces_nothing() {
    let fake = FakeHerdr::new();
    let session = session_with(&fake, &["api"]);
    let mut watcher = Client::connect(&session);
    watcher.subscribe("w", json!([{ "type": "workspace.created" }]));
    let mut client = Client::connect(&session);
    session.reject_next_creation("server_busy", "try again later");

    client.send("c", "workspace.create", json!({}));
    let response = client.read().expect("a response");

    assert_eq!(response["error"]["code"], "server_busy");
    assert_eq!(client.snapshot_labels(), ["api"]);
    assert_eq!(session.emit_event("workspace_created", json!({})), 1);
    assert_eq!(
        watcher.read().expect("the probe event")["data"],
        json!({}),
        "nothing was announced before the probe"
    );
}

fn pane_ids(client: &mut Client) -> Vec<String> {
    client.send("p", "pane.list", json!({}));
    let response = client.read().expect("a pane list");
    response["result"]["panes"]
        .as_array()
        .expect("panes")
        .iter()
        .map(|pane| pane["pane_id"].as_str().expect("pane_id").to_owned())
        .collect()
}

fn session_with_two_tabs(fake: &FakeHerdr) -> FakeSession {
    fake.start_session(
        "default",
        SessionScript::new().workspace_with_tabs(
            "api",
            [
                TabScript::new(
                    "one",
                    PaneTree::split(
                        SplitDirection::Right,
                        0.5,
                        PaneTree::pane(),
                        PaneTree::pane(),
                    ),
                ),
                TabScript::new("two", PaneTree::pane()),
            ],
        ),
    )
}

#[test]
fn closing_a_pane_announces_pane_closed_and_the_others_keep_their_ids_and_the_freed_space() {
    let fake = FakeHerdr::new();
    let session = session_with_two_tabs(&fake);
    let mut watcher = Client::connect(&session);
    watcher.subscribe("w", json!([{ "type": "pane.closed" }]));
    let mut client = Client::connect(&session);

    client.send("c", "pane.close", json!({ "pane_id": "w1:p1" }));
    let response = client.read().expect("a response");

    assert_eq!(response["result"], json!({ "type": "ok" }));
    let event = watcher.read().expect("an event");
    assert_eq!(event["event"], "pane_closed");
    assert_eq!(
        event["data"],
        json!({ "type": "pane_closed", "pane_id": "w1:p1", "workspace_id": "w1" })
    );
    assert_eq!(pane_ids(&mut client), ["w1:p2", "w1:p3"]);
    client.send("s", "session.snapshot", json!({}));
    let snapshot = client.read().expect("a snapshot");
    let layout = &snapshot["result"]["snapshot"]["layouts"][0];
    assert_eq!(layout["panes"][0]["pane_id"], "w1:p2");
    assert_eq!(layout["panes"][0]["rect"], layout["area"]);
    assert_eq!(layout["panes"][0]["focused"], true);
}

#[test]
fn closing_the_only_pane_of_a_herdr_tab_closes_the_tab_and_of_a_workspace_the_workspace() {
    let fake = FakeHerdr::new();
    let session = session_with_two_tabs(&fake);
    let mut client = Client::connect(&session);

    client.send("c", "pane.close", json!({ "pane_id": "w1:p3" }));
    client.read().expect("a response");
    client.send("s", "session.snapshot", json!({}));
    let snapshot = client.read().expect("a snapshot");
    assert_eq!(
        snapshot["result"]["snapshot"]["tabs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    client.send("c", "pane.close", json!({ "pane_id": "w1:p1" }));
    client.read().expect("a response");
    client.send("c", "pane.close", json!({ "pane_id": "w1:p2" }));
    client.read().expect("a response");
    assert_eq!(client.snapshot_labels(), Vec::<String>::new());
}

#[test]
fn a_pane_that_does_not_exist_or_a_rejected_close_changes_nothing_and_announces_nothing() {
    let fake = FakeHerdr::new();
    let session = session_with_two_tabs(&fake);
    let mut watcher = Client::connect(&session);
    watcher.subscribe("w", json!([{ "type": "pane.closed" }]));
    let mut client = Client::connect(&session);

    client.send("c", "pane.close", json!({ "pane_id": "w9:p1" }));
    let missing = client.read().expect("a response");
    session.reject_next_close("pane_busy", "try again later");
    client.send("c", "pane.close", json!({ "pane_id": "w1:p1" }));
    let rejected = client.read().expect("a response");

    assert_eq!(missing["error"]["code"], "pane_not_found");
    assert_eq!(missing["error"]["message"], "pane w9:p1 not found");
    assert_eq!(rejected["error"]["code"], "pane_busy");
    assert_eq!(pane_ids(&mut client), ["w1:p1", "w1:p2", "w1:p3"]);
    assert_eq!(session.emit_event("pane_closed", json!({})), 1);
    assert_eq!(
        watcher.read().expect("the probe event")["data"],
        json!({}),
        "nothing was announced before the probe"
    );
}

#[test]
fn a_scripted_pane_name_is_reported_as_the_panes_label() {
    let fake = FakeHerdr::new();
    let session = fake.start_session(
        "default",
        SessionScript::new().workspace_with_tabs(
            "api",
            [TabScript::new(
                "one",
                PaneTree::split(
                    SplitDirection::Right,
                    0.5,
                    PaneTree::pane(),
                    PaneTree::pane(),
                ),
            )
            .pane_label(1, "build")],
        ),
    );
    let mut client = Client::connect(&session);

    client.send("p", "pane.list", json!({}));
    let panes = client.read().expect("a pane list")["result"]["panes"].clone();

    assert_eq!(panes[0].get("label"), None);
    assert_eq!(panes[1]["label"], "build");
}
