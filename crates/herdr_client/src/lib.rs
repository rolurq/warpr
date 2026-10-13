//! Client for a Herdr session's JSON socket API.
//!
//! The crate turns what a session reports into a [`PresentedSession`], the state a front-end
//! renders. It owns no UI and no async runtime: calls block, so callers run them off their UI
//! thread.
//!
//! Every call addresses the socket of an explicit [`SessionTarget`]. Herdr's own
//! `HERDR_SOCKET_PATH` points at whichever session the launching shell lives in, so it is never
//! consulted.

mod cli;
mod presented;
mod session;
mod socket;

pub use cli::{HERDR_BINARY_ENV, HerdrCli};
pub use presented::{
    ConnectionFailure, ConnectionStatus, PresentedSession, PresentedWorkspace, WorkspaceId,
};
pub use session::{DEFAULT_SESSION, InvalidSessionName, SessionName, SessionTarget, config_dir};

use serde::Deserialize;
use serde_json::json;

/// Connects to `target` and presents its current workspaces.
///
/// Failure to connect is reported in [`PresentedSession::connection`] rather than as an error,
/// because a front-end shows it as part of the presented state.
pub fn present_session(target: &SessionTarget) -> PresentedSession {
    match fetch_snapshot(target) {
        Ok(snapshot) => PresentedSession::from_snapshot(snapshot),
        Err(failure) => PresentedSession::failed(failure),
    }
}

#[derive(Deserialize)]
struct SnapshotResult {
    snapshot: presented::Snapshot,
}

fn fetch_snapshot(target: &SessionTarget) -> Result<presented::Snapshot, ConnectionFailure> {
    let result = socket::request(target.socket_path(), "session.snapshot", json!({}))?;
    let SnapshotResult { snapshot } = serde_json::from_value(result)
        .map_err(|error| ConnectionFailure::Protocol(error.to_string()))?;
    Ok(snapshot)
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
