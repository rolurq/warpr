use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

use command::blocking::Command;
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
    send(
        &mut stream,
        json!({"id": "x", "method": "nope", "params": {}}),
    );
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

#[test]
fn concurrent_invocations_are_each_recorded_whole() {
    const CONCURRENT: usize = 48;
    let mut fake = FakeHerdr::new();
    fake.script_command(&["pane", "list"], "", 0);

    let runs: Vec<_> = (0..CONCURRENT)
        .map(|_| {
            let binary = fake.binary_path().to_owned();
            std::thread::spawn(move || {
                Command::new(binary)
                    .args(["pane", "list"])
                    .output()
                    .unwrap()
            })
        })
        .collect();
    for run in runs {
        run.join().unwrap();
    }

    let invocations = fake.invocations();
    assert_eq!(invocations.len(), CONCURRENT);
    assert!(
        invocations
            .iter()
            .all(|invocation| invocation.args == ["pane", "list"])
    );
}

#[test]
fn scripted_tabs_are_reported_with_herdr_ids_and_split_geometry() {
    let fake = FakeHerdr::new();
    let session = fake.start_session(
        "s",
        SessionScript::new().workspace_with_tabs(
            "a",
            [TabScript::new(
                "t",
                PaneTree::split(
                    SplitDirection::Right,
                    0.3,
                    PaneTree::pane(),
                    PaneTree::pane(),
                ),
            )
            .focused_pane(1)
            .zoomed()],
        ),
    );

    let mut stream = UnixStream::connect(session.socket_path()).unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    send(
        &mut stream,
        json!({"id": "1", "method": "session.snapshot", "params": {}}),
    );
    let snapshot = read_json(&mut reader)["result"]["snapshot"].clone();

    assert_eq!(snapshot["tabs"][0]["tab_id"], "w1:t1");
    assert_eq!(snapshot["workspaces"][0]["active_tab_id"], "w1:t1");
    assert_eq!(snapshot["focused_pane_id"], "w1:p2");
    let layout = &snapshot["layouts"][0];
    assert_eq!(layout["zoomed"], true);
    assert_eq!(layout["focused_pane_id"], "w1:p2");
    assert_eq!(
        layout["panes"][1]["rect"],
        json!({"x": 36, "y": 0, "width": 84, "height": 40})
    );
    assert_eq!(layout["splits"][0]["id"], "split_0_root");
}

#[test]
fn served_methods_lists_requests_in_arrival_order() {
    let fake = FakeHerdr::new();
    let session = fake.start_session("s", SessionScript::new());

    let mut stream = UnixStream::connect(session.socket_path()).unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    for method in ["ping", "session.snapshot"] {
        send(
            &mut stream,
            json!({"id": "1", "method": method, "params": {}}),
        );
        read_json(&mut reader);
    }

    assert_eq!(session.served_methods(), ["ping", "session.snapshot"]);
}

#[test]
fn workspace_ids_survive_removing_and_renaming_other_workspaces() {
    let fake = FakeHerdr::new();
    let script = SessionScript::new()
        .workspace("a")
        .workspace("b")
        .workspace("c")
        .without_workspace("a")
        .renamed_workspace("c", "d");
    let session = fake.start_session("s", script);

    let mut stream = UnixStream::connect(session.socket_path()).unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    send(
        &mut stream,
        json!({"id": "1", "method": "session.snapshot", "params": {}}),
    );
    let snapshot = read_json(&mut reader)["result"]["snapshot"].clone();

    let workspaces: Vec<_> = snapshot["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|workspace| {
            (
                workspace["workspace_id"].clone(),
                workspace["label"].clone(),
            )
        })
        .collect();
    assert_eq!(
        workspaces,
        [(json!("w2"), json!("b")), (json!("w3"), json!("d"))]
    );
    assert_eq!(snapshot["focused_workspace_id"], "w2");
}

#[test]
fn scripted_session_list_has_herdrs_shape_and_order() {
    let mut fake = FakeHerdr::new();
    fake.script_session_list(&[("default", true), ("work", false)]);

    let output = Command::new(fake.binary_path())
        .args(["session", "list", "--json"])
        .output()
        .unwrap();

    let listing: Value = serde_json::from_slice(&output.stdout).unwrap();
    let sessions = listing["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0]["name"], "default");
    assert_eq!(sessions[0]["default"], true);
    assert_eq!(sessions[0]["running"], true);
    assert_eq!(sessions[1]["name"], "work");
    assert_eq!(sessions[1]["default"], false);
    assert_eq!(sessions[1]["running"], false);
    assert_eq!(
        sessions[1]["socket_path"],
        fake.config_dir()
            .join("sessions/work/herdr.sock")
            .to_str()
            .unwrap()
    );
}

#[test]
fn running_the_scripted_server_command_brings_the_session_up() {
    let mut fake = FakeHerdr::new();
    fake.script_server_start("work", SessionScript::new().workspace("a"));
    let socket_path = fake.config_dir().join("sessions/work/herdr.sock");
    assert!(UnixStream::connect(&socket_path).is_err());

    let mut server = Command::new(fake.binary_path())
        .args(["--session", "work", "server"])
        .spawn()
        .unwrap();

    let mut connected = None;
    for _ in 0..200 {
        if let Ok(stream) = UnixStream::connect(&socket_path) {
            connected = Some(stream);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let mut stream = connected.expect("session never came up");
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    send(
        &mut stream,
        json!({"id": "1", "method": "session.snapshot", "params": {}}),
    );
    assert_eq!(
        read_json(&mut reader)["result"]["snapshot"]["workspaces"][0]["label"],
        "a"
    );
    assert!(
        server.try_wait().unwrap().is_none(),
        "a server keeps running"
    );
    server.kill().unwrap();
}

#[test]
fn scripting_a_command_again_replaces_what_it_did_before() {
    let mut fake = FakeHerdr::new();
    fake.script_command(&["status", "server"], "first\n", 0);
    fake.script_command(&["status", "server"], "second\n", 3);

    let output = Command::new(fake.binary_path())
        .args(["status", "server"])
        .output()
        .unwrap();

    assert_eq!(String::from_utf8_lossy(&output.stdout), "second\n");
    assert_eq!(output.status.code(), Some(3));
}
