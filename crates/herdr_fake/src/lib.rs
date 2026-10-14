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
mod script;
mod server;

use std::fs;
use std::path::{Path, PathBuf};

pub use binary::Invocation;
use binary::ScriptedBinary;
pub use script::{PaneTree, SessionScript, SplitDirection, TabScript};
pub use server::FakeSession;
use tempfile::TempDir;

/// A scratch Herdr config directory plus a scripted `herdr` executable.
pub struct FakeHerdr {
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
        Self { root, binary }
    }

    /// Herdr's config directory, laid out as the real one is: the default session's socket sits in
    /// it and every other session's in `sessions/<name>/`.
    pub fn config_dir(&self) -> PathBuf {
        self.root.path().join("herdr")
    }

    /// Starts a server for session `name` that answers from `script`, listening where a real
    /// Herdr session of that name would. It stops when dropped.
    pub fn start_session(&self, name: &str, script: SessionScript) -> FakeSession {
        let config_dir = self.config_dir();
        let socket_path = if name == "default" {
            config_dir.join("herdr.sock")
        } else {
            config_dir.join("sessions").join(name).join("herdr.sock")
        };
        fs::create_dir_all(socket_path.parent().expect("socket path has a parent"))
            .expect("create session directory");
        FakeSession::start(socket_path, script)
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
