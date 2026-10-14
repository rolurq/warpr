use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::binary::read_invocations;
use crate::{FakeSession, SessionScript};

const POLL_INTERVAL: Duration = Duration::from_millis(5);

struct Pending {
    server_args: Vec<String>,
    socket_path: PathBuf,
    script: SessionScript,
}

/// Brings a fake session's socket up once the scripted `herdr` is run to start its server.
///
/// The scripted executable is a shell script and cannot serve a socket itself, so a thread in the
/// test process watches its invocation log and starts the [`FakeSession`] when the matching
/// server command shows up, as a real server would start listening.
pub(crate) struct ServerLauncher {
    pending: Arc<Mutex<Vec<Pending>>>,
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ServerLauncher {
    pub(crate) fn start(invocation_log: PathBuf) -> Self {
        let pending = Arc::new(Mutex::new(Vec::<Pending>::new()));
        let stopping = Arc::new(AtomicBool::new(false));
        let thread = thread::spawn({
            let pending = Arc::clone(&pending);
            let stopping = Arc::clone(&stopping);
            move || {
                // Held here so the sessions stop, and their sockets disappear, with the launcher.
                let mut started = Vec::<FakeSession>::new();
                while !stopping.load(Ordering::SeqCst) {
                    let invocations = read_invocations(&invocation_log);
                    let mut pending = pending.lock().expect("pending servers lock");
                    let (ready, waiting): (Vec<_>, Vec<_>) =
                        pending.drain(..).partition(|server| {
                            invocations
                                .iter()
                                .any(|invocation| invocation.args == server.server_args)
                        });
                    *pending = waiting;
                    drop(pending);
                    for server in ready {
                        started.push(FakeSession::start(server.socket_path, server.script));
                    }
                    thread::sleep(POLL_INTERVAL);
                }
            }
        });
        Self {
            pending,
            stopping,
            thread: Some(thread),
        }
    }

    pub(crate) fn serve_when_started(
        &self,
        server_args: Vec<String>,
        socket_path: PathBuf,
        script: SessionScript,
    ) {
        self.pending
            .lock()
            .expect("pending servers lock")
            .push(Pending {
                server_args,
                socket_path,
                script,
            });
    }
}

impl Drop for ServerLauncher {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
