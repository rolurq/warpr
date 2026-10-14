//! Herdr's sessions as a whole: which exist, which are running, and starting a stopped one.

use std::io;
use std::process::Output;

use command::blocking::Command;
use serde::Deserialize;

use crate::{
    ConnectionFailure, HerdrCli, PresentedSession, STATUS_TIMEOUT, SessionName, SessionTarget,
    compat, present_session, socket,
};

/// Whether a session's server is up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    Running,
    Stopped,
}

/// One entry of Herdr's session listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedSession {
    pub name: SessionName,
    pub state: SessionState,
}

#[derive(Deserialize)]
struct Listing {
    sessions: Vec<ListedEntry>,
}

/// The part of a `herdr session list --json` entry this client reads. The entry also carries the
/// session's directory and socket path, which are derived from the name instead so every call
/// addresses a socket by one rule.
#[derive(Deserialize)]
struct ListedEntry {
    name: String,
    running: bool,
}

/// Variables Herdr sets for processes inside one of its panes, which would redirect a command.
const INHERITED_PANE_VARS: [&str; 3] = ["HERDR_PANE_ID", "HERDR_TAB_ID", "HERDR_WORKSPACE_ID"];

fn command(cli: &HerdrCli, args: &[&str]) -> Command {
    let mut command = Command::new(cli.binary());
    command.args(args);
    for var in INHERITED_PANE_VARS {
        command.env_remove(var);
    }
    command
}

/// Runs a command that is about every session rather than one. The session this process
/// inherited is dropped so it cannot narrow the answer.
fn run_unscoped(cli: &HerdrCli, args: &[&str]) -> io::Result<Output> {
    command(cli, args)
        .env_remove("HERDR_SOCKET_PATH")
        .env_remove("HERDR_SESSION")
        .output()
}

/// Starts `herdr <args>` for `target` as the leader of its own session, so that it neither
/// receives the hangups and interrupts aimed at this process's terminal nor ends with it. Its
/// stdio is detached from this process.
#[cfg(unix)]
fn spawn_detached(
    cli: &HerdrCli,
    target: &SessionTarget,
    args: &[&str],
) -> io::Result<std::process::Child> {
    let mut command = command(cli, args);
    command
        .env("HERDR_SOCKET_PATH", target.socket_path())
        .env("HERDR_SESSION", target.name().as_str());
    // SAFETY: the hook calls only `setsid`, which is async-signal-safe.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command.spawn()
}

/// Asks Herdr which sessions exist, in the order it lists them.
pub fn list_sessions(cli: &HerdrCli) -> Result<Vec<ListedSession>, ConnectionFailure> {
    let unavailable = |reason: String| ConnectionFailure::SessionListUnavailable { reason };
    let output = run_unscoped(cli, &["session", "list", "--json"])
        .map_err(|error| unavailable(error.to_string()))?;
    if !output.status.success() {
        return Err(unavailable(format!(
            "`herdr session list` exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let listing: Listing = serde_json::from_slice(&output.stdout)
        .map_err(|error| unavailable(format!("unreadable listing: {error}")))?;
    listing
        .sessions
        .into_iter()
        .map(|entry| {
            let name =
                SessionName::new(entry.name).map_err(|error| unavailable(error.to_string()))?;
            let state = if entry.running {
                SessionState::Running
            } else {
                SessionState::Stopped
            };
            Ok(ListedSession { name, state })
        })
        .collect()
}

/// Connects to `target` after bringing its server up when `state` says it is stopped.
///
/// Failure is reported in [`PresentedSession::connection`], as [`present_session`] does.
pub fn open_session(
    cli: &HerdrCli,
    target: &SessionTarget,
    state: SessionState,
) -> PresentedSession {
    if state == SessionState::Stopped
        && let Err(failure) = start_server(cli, target)
    {
        return PresentedSession::failed(failure);
    }
    present_session(cli, target)
}

/// Connects to `target` for the first time on launch, asking Herdr whether its server is up.
///
/// The default session is started when it is down. Any other session must already exist in
/// Herdr's listing: a mistyped name is reported rather than creating a session.
pub fn launch_session(cli: &HerdrCli, target: &SessionTarget) -> PresentedSession {
    // Without this, a missing or broken `herdr` would surface as a failed session listing
    // instead of the refusal that says what to install or fix.
    if let Err(refusal) = compat::check(cli, target, STATUS_TIMEOUT) {
        return PresentedSession::refused(refusal);
    }
    let sessions = match list_sessions(cli) {
        Ok(sessions) => sessions,
        Err(failure) => return PresentedSession::failed(failure),
    };
    let listed = sessions
        .iter()
        .find(|session| &session.name == target.name());
    let state = match listed {
        Some(session) => session.state,
        None if target.name() == &SessionName::default_session() => SessionState::Stopped,
        None => {
            return PresentedSession::failed(ConnectionFailure::SessionUnresolvable {
                name: target.name().as_str().to_owned(),
                reason: "Herdr does not list a session with this name".to_owned(),
            });
        }
    };
    open_session(cli, target, state)
}

/// Starts the headless server of `target` and waits until it accepts connections.
///
/// The server is detached from this process, so it keeps running, panes included, after this
/// process exits. Nothing in this crate ever stops one.
#[cfg(unix)]
fn start_server(cli: &HerdrCli, target: &SessionTarget) -> Result<(), ConnectionFailure> {
    use std::time::Duration;

    const READY_POLL_INTERVAL: Duration = Duration::from_millis(50);
    /// How long a freshly started server may take before it accepts connections: 15 seconds.
    const READY_POLLS: u32 = 300;

    let failed = |reason: String| ConnectionFailure::ServerStartFailed {
        name: target.name().as_str().to_owned(),
        reason,
    };

    // Herdr runs the default session when no name is given.
    let named_args = ["--session", target.name().as_str(), "server"];
    let args: &[&str] = if target.name() == &SessionName::default_session() {
        &["server"]
    } else {
        &named_args
    };
    let mut server = spawn_detached(cli, target, args)
        .map_err(|error| failed(format!("could not run herdr: {error}")))?;

    let mut polls = 0;
    let result = loop {
        match server.try_wait() {
            Ok(Some(status)) => break Err(failed(format!("the server exited with {status}"))),
            Ok(None) => {}
            Err(error) => break Err(failed(format!("could not watch the server: {error}"))),
        }
        if socket::request(target.socket_path(), "ping", serde_json::json!({})).is_ok() {
            break Ok(());
        }
        polls += 1;
        if polls >= READY_POLLS {
            break Err(failed(format!(
                "the server did not accept connections within {} seconds",
                (READY_POLL_INTERVAL * READY_POLLS).as_secs()
            )));
        }
        std::thread::sleep(READY_POLL_INTERVAL);
    };

    // Dropping a `Child` neither waits for nor kills it. Waiting on a thread reaps the server if it
    // ever exits while this process lives, instead of leaving a zombie.
    std::thread::spawn(move || {
        let _ = server.wait();
    });
    result
}

#[cfg(not(unix))]
fn start_server(_cli: &HerdrCli, _target: &SessionTarget) -> Result<(), ConnectionFailure> {
    Err(ConnectionFailure::UnsupportedPlatform)
}

#[cfg(test)]
#[path = "sessions_tests.rs"]
mod tests;
