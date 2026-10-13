use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::Command;

use serde_json::{Value, json};

use super::*;

fn send(stream: &mut UnixStream, request: Value) {
    writeln!(stream, "{request}").expect("send request");
}

fn read_json(reader: &mut BufReader<UnixStream>) -> Value {
    let mut line = String::new();
    reader.read_line(&mut line).expect("read line");
    serde_json::from_str(&line).expect("valid json line")
}

#[test]
fn session_listens_in_herdr_layout_and_answers_snapshot() {
    let fake = FakeHerdr::new();
    let default = fake.start_session("default", SessionScript::new().workspace("main"));
    let work = fake.start_session("work", SessionScript::new().workspace("a").workspace("b"));

    assert_eq!(default.socket_path(), fake.config_dir().join("herdr.sock"));
    assert_eq!(
        work.socket_path(),
        fake.config_dir().join("sessions/work/herdr.sock")
    );

    let mut stream = UnixStream::connect(work.socket_path()).unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    send(
        &mut stream,
        json!({"id": "1", "method": "session.snapshot", "params": {}}),
    );
    let response = read_json(&mut reader);

    assert_eq!(response["id"], "1");
    let snapshot = &response["result"]["snapshot"];
    assert_eq!(snapshot["focused_workspace_id"], "w1");
    let labels: Vec<_> = snapshot["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|workspace| workspace["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, ["a", "b"]);
}

#[test]
fn focused_script_choice_is_reported_in_snapshot() {
    let fake = FakeHerdr::new();
    let session = fake.start_session(
        "s",
        SessionScript::new()
            .workspace("a")
            .workspace("b")
            .focused("b"),
    );

    let mut stream = UnixStream::connect(session.socket_path()).unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    send(
        &mut stream,
        json!({"id": "1", "method": "session.snapshot", "params": {}}),
    );
    let snapshot = read_json(&mut reader)["result"]["snapshot"].clone();

    assert_eq!(snapshot["focused_workspace_id"], "w2");
    assert_eq!(snapshot["workspaces"][0]["focused"], false);
    assert_eq!(snapshot["workspaces"][1]["focused"], true);
}

#[test]
fn unknown_methods_get_an_error_response() {
    let fake = FakeHerdr::new();
    let session = fake.start_session("s", SessionScript::new());

    let mut stream = UnixStream::connect(session.socket_path()).unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    send(&mut stream, json!({"id": "x", "method": "nope", "params": {}}));
    let response = read_json(&mut reader);

    assert_eq!(response["id"], "x");
    assert_eq!(response["error"]["code"], "invalid_request");
}

#[test]
fn subscribers_receive_emitted_events_after_the_subscription_response() {
    let fake = FakeHerdr::new();
    let session = fake.start_session("s", SessionScript::new());

    let mut stream = UnixStream::connect(session.socket_path()).unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    send(
        &mut stream,
        json!({
            "id": "sub",
            "method": "events.subscribe",
            "params": {"subscriptions": [{"type": "workspace.renamed"}]},
        }),
    );
    assert_eq!(
        read_json(&mut reader),
        json!({"id": "sub", "result": {"type": "subscription_started"}})
    );

    let data = json!({"type": "workspace_renamed", "workspace_id": "w1", "label": "new"});
    session.emit_event("workspace_renamed", data.clone());

    assert_eq!(
        read_json(&mut reader),
        json!({"event": "workspace_renamed", "data": data})
    );
}

#[test]
fn dropping_a_session_removes_its_socket() {
    let fake = FakeHerdr::new();
    let session = fake.start_session("s", SessionScript::new());
    let socket_path = session.socket_path().to_owned();

    drop(session);

    assert!(!socket_path.exists());
}

#[test]
fn scripted_binary_prints_scripted_stdout_and_exit_code() {
    let mut fake = FakeHerdr::new();
    fake.script_command(&["session", "list", "--json"], "[\"default\"]\n", 0);
    fake.script_command(&["status", "server"], "not running\n", 3);

    let listed = Command::new(fake.binary_path())
        .args(["session", "list", "--json"])
        .output()
        .unwrap();
    let status = Command::new(fake.binary_path())
        .args(["status", "server"])
        .output()
        .unwrap();

    assert_eq!(String::from_utf8_lossy(&listed.stdout), "[\"default\"]\n");
    assert!(listed.status.success());
    assert_eq!(String::from_utf8_lossy(&status.stdout), "not running\n");
    assert_eq!(status.status.code(), Some(3));
}

#[test]
fn unscripted_invocation_fails_loudly() {
    let fake = FakeHerdr::new();

    let output = Command::new(fake.binary_path())
        .arg("surprise")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(64));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unscripted invocation: surprise"));
}

#[test]
fn invocations_record_args_and_herdr_environment() {
    let mut fake = FakeHerdr::new();
    fake.script_command(&["pane", "list"], "", 0);

    Command::new(fake.binary_path())
        .args(["pane", "list"])
        .env("HERDR_SESSION", "work")
        .env_remove("HERDR_PANE_ID")
        .output()
        .unwrap();

    let invocations = fake.invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].args, ["pane", "list"]);
    assert_eq!(invocations[0].env("HERDR_SESSION"), Some("work"));
    assert_eq!(invocations[0].env("HERDR_PANE_ID"), None);
}
