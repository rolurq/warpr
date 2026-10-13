use std::io;
use std::path::PathBuf;
use std::process::Output;

use command::blocking::Command;

use crate::SessionTarget;

/// Environment variable naming the `herdr` executable to run instead of the one on `PATH`.
pub const HERDR_BINARY_ENV: &str = "WARPR_HERDR_BIN";

/// Variables Herdr sets for processes inside one of its panes. They identify that pane's
/// workspace, tab and pane, and would silently redirect a command to them.
const INHERITED_PANE_VARS: [&str; 3] = ["HERDR_PANE_ID", "HERDR_TAB_ID", "HERDR_WORKSPACE_ID"];

/// Runs the `herdr` executable against an explicit session.
#[derive(Clone, Debug)]
pub struct HerdrCli {
    binary: PathBuf,
}

impl HerdrCli {
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
        }
    }

    /// Uses [`HERDR_BINARY_ENV`] when set and `herdr` from `PATH` otherwise.
    pub fn from_env() -> Self {
        Self::new(std::env::var_os(HERDR_BINARY_ENV).unwrap_or_else(|| "herdr".into()))
    }

    /// Runs `herdr <args>` for `target`, overriding any session this process inherited.
    pub fn run(&self, target: &SessionTarget, args: &[&str]) -> io::Result<Output> {
        let mut command = Command::new(&self.binary);
        command
            .args(args)
            .env("HERDR_SOCKET_PATH", target.socket_path())
            .env("HERDR_SESSION", target.name().as_str());
        for var in INHERITED_PANE_VARS {
            command.env_remove(var);
        }
        command.output()
    }
}
