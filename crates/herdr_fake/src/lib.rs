//! A fake Herdr for tests.
//!
//! It stands in for the two surfaces a Herdr client talks to, so client and presentation code
//! can be exercised without a real Herdr or a display:
//!
//! - [`FakeSession`]: a socket server answering the JSON socket API from a [`SessionScript`];
//! - [`FakeHerdr::binary_path`]: a `herdr` executable that prints scripted output and records how
//!   it was invoked.
//!
//! Responses mirror the wire format of Herdr 0.9.3 (protocol 22) as captured from a real session.

#![cfg(unix)]

mod binary;
mod launcher;
mod script;
mod server;
mod status;

use std::fs;
use std::path::{Path, PathBuf};

pub use binary::Invocation;
use binary::ScriptedBinary;
use launcher::ServerLauncher;
pub use script::{PaneTree, SessionScript, SplitDirection, TabScript};
pub use server::FakeSession;
pub use status::HerdrStatus;
use tempfile::TempDir;

/// A scratch Herdr config directory plus a scripted `herdr` executable.
pub struct FakeHerdr {
    // Declared first so the launcher's sessions stop before the directory they sit in goes away.
    launcher: Option<ServerLauncher>,
    root: TempDir,
    binary: ScriptedBinary,
}

impl FakeHerdr {
    pub fn new() -> Self {
        // Unix socket paths are limited to ~100 bytes, so keep the root's name short.
        let root = tempfile::Builder::new()
            .prefix("hf")
            .tempdir()
            .expect("create fake Herdr directory");
        let binary = ScriptedBinary::new(root.path().join("bin").join("herdr"));
        Self {
            launcher: None,
            root,
            binary,
        }
    }

    /// Herdr's config directory, laid out as the real one is: the default session's socket sits in
    /// it and every other session's in `sessions/<name>/`.
    pub fn config_dir(&self) -> PathBuf {
        self.root.path().join("herdr")
    }

    /// Starts a server for session `name` that answers from `script`, listening where a real
    /// Herdr session of that name would. It stops when dropped.
    pub fn start_session(&self, name: &str, script: SessionScript) -> FakeSession {
        FakeSession::start(self.prepare_socket_path(name), script)
    }

    fn prepare_socket_path(&self, name: &str) -> PathBuf {
        let config_dir = self.config_dir();
        let socket_path = if name == "default" {
            config_dir.join("herdr.sock")
        } else {
            config_dir.join("sessions").join(name).join("herdr.sock")
        };
        fs::create_dir_all(socket_path.parent().expect("socket path has a parent"))
            .expect("create session directory");
        socket_path
    }

    /// The scripted `herdr` executable. It exits with status 64 on an invocation that was not
    /// scripted, so tests fail loudly instead of reading empty output.
    pub fn binary_path(&self) -> &Path {
        self.binary.path()
    }

    /// Makes `herdr <args>` print `stdout` and exit with `exit_code`.
    pub fn script_command(&mut self, args: &[&str], stdout: &str, exit_code: i32) {
        self.binary.script(args, stdout, exit_code);
    }

    /// Makes `herdr session list --json` report `sessions` as `(name, running)`, in that order,
    /// in the shape Herdr 0.9.3 prints. It reports only what is given here; it does not look at
    /// which fake sessions are started.
    pub fn script_session_list(&mut self, sessions: &[(&str, bool)]) {
        let config_dir = self.config_dir();
        let entries: Vec<_> = sessions
            .iter()
            .map(|(name, running)| {
                let is_default = *name == "default";
                let session_dir = if is_default {
                    config_dir.clone()
                } else {
                    config_dir.join("sessions").join(name)
                };
                serde_json::json!({
                    "default": is_default,
                    "name": name,
                    "running": running,
                    "session_dir": session_dir,
                    "socket_path": session_dir.join("herdr.sock"),
                })
            })
            .collect();
        let listing = serde_json::json!({ "sessions": entries });
        self.binary
            .script(&["session", "list", "--json"], &format!("{listing}\n"), 0);
    }

    /// Makes running the server command for session `name` (`herdr --session <name> server`, or
    /// `herdr server` for the default session) behave like starting a real server: the scripted
    /// executable keeps running until this fake is dropped, and the session starts answering from
    /// `script` on its socket.
    pub fn script_server_start(&mut self, name: &str, script: SessionScript) {
        let server_args: Vec<String> = if name == "default" {
            vec!["server".to_owned()]
        } else {
            ["--session", name, "server"].map(str::to_owned).to_vec()
        };
        let args: Vec<&str> = server_args.iter().map(String::as_str).collect();
        self.binary.script_until_shutdown(&args);
        let socket_path = self.prepare_socket_path(name);
        let log = self.binary.invocation_log();
        self.launcher
            .get_or_insert_with(|| ServerLauncher::start(log))
            .serve_when_started(server_args, socket_path, script);
    }

    /// Makes `herdr status server --json` and `herdr status client --json` print what `status`
    /// describes, the way Herdr 0.9.3 prints them.
    pub fn script_status(&mut self, status: &HerdrStatus) {
        self.binary.script(
            &["status", "server", "--json"],
            &format!("{}\n", status.server_json()),
            0,
        );
        self.binary.script(
            &["status", "client", "--json"],
            &format!("{}\n", status.client_json()),
            0,
        );
    }

    /// Makes `herdr <args>` run forever without printing anything.
    pub fn script_hang(&mut self, args: &[&str]) {
        self.binary.script_hang(args);
    }

    /// Every invocation of the scripted executable so far, in order.
    pub fn invocations(&self) -> Vec<Invocation> {
        self.binary.invocations()
    }
}

impl Default for FakeHerdr {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
