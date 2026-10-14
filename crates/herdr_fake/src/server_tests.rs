use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use serde_json::{Value, json};

use super::*;
use crate::FakeHerdr;

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
