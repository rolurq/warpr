use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use serde_json::{Value, json};

use crate::SessionScript;

struct Shared {
    script: SessionScript,
    subscribers: Mutex<Vec<UnixStream>>,
    requests_served: AtomicUsize,
    methods_served: Mutex<Vec<String>>,
    stopping: AtomicBool,
}

/// A running fake Herdr session server. Stops and removes its socket when dropped.
pub struct FakeSession {
    socket_path: PathBuf,
    shared: Arc<Shared>,
    accept_thread: Option<JoinHandle<()>>,
}

impl FakeSession {
    pub(crate) fn start(socket_path: PathBuf, script: SessionScript) -> Self {
        let listener = UnixListener::bind(&socket_path)
            .unwrap_or_else(|error| panic!("bind {}: {error}", socket_path.display()));
        let shared = Arc::new(Shared {
            script,
            subscribers: Mutex::new(Vec::new()),
            requests_served: AtomicUsize::new(0),
            methods_served: Mutex::new(Vec::new()),
            stopping: AtomicBool::new(false),
        });
        let accept_thread = thread::spawn({
            let shared = Arc::clone(&shared);
            move || {
                for stream in listener.incoming() {
                    if shared.stopping.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    let shared = Arc::clone(&shared);
                    thread::spawn(move || serve_connection(stream, &shared));
                }
            }
        });
        Self {
            socket_path,
            shared,
            accept_thread: Some(accept_thread),
        }
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
        self.shared
            .methods_served
            .lock()
            .expect("methods lock")
            .clone()
    }

    /// Pushes an event to every connection that sent `events.subscribe`, framed as Herdr frames
    /// it: `{"event": <name>, "data": <data>}`.
    pub fn emit_event(&self, name: &str, data: Value) {
        let line = format!("{}\n", json!({ "event": name, "data": data }));
        let mut subscribers = self.shared.subscribers.lock().expect("subscribers lock");
        subscribers.retain_mut(|stream| stream.write_all(line.as_bytes()).is_ok());
    }
}

impl Drop for FakeSession {
    fn drop(&mut self) {
        self.shared.stopping.store(true, Ordering::SeqCst);
        // Unblocks the accept loop so it can observe `stopping`.
        let _ = UnixStream::connect(&self.socket_path);
        if let Some(thread) = self.accept_thread.take() {
            let _ = thread.join();
        }
        for stream in self
            .shared
            .subscribers
            .lock()
            .expect("subscribers lock")
            .drain(..)
        {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

fn serve_connection(stream: UnixStream, shared: &Shared) {
    let Ok(reader_stream) = stream.try_clone() else {
        return;
    };
    let mut writer = stream;
    for line in BufReader::new(reader_stream).lines() {
        let Ok(line) = line else { return };
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            return;
        };
        let id = request["id"].clone();
        let method = request["method"].as_str().unwrap_or_default();
        shared
            .methods_served
            .lock()
            .expect("methods lock")
            .push(method.to_owned());
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
            "session.snapshot" => ok(
                &id,
                json!({ "type": "session_snapshot", "snapshot": shared.script.snapshot() }),
            ),
            "events.subscribe" => {
                // Registering under the lock keeps an emitted event from overtaking the response.
                let mut subscribers = shared.subscribers.lock().expect("subscribers lock");
                let response = ok(&id, json!({ "type": "subscription_started" }));
                if writeln!(writer, "{response}").is_err() {
                    return;
                }
                match writer.try_clone() {
                    Ok(subscriber) => subscribers.push(subscriber),
                    Err(_) => return,
                }
                continue;
            }
            other => json!({
                "id": id,
                "error": {
                    "code": "invalid_request",
                    "message": format!("invalid request: unknown variant `{other}`"),
                },
            }),
        };
        if writeln!(writer, "{response}").is_err() {
            return;
        }
    }
}

fn ok(id: &Value, result: Value) -> Value {
    json!({ "id": id, "result": result })
}
