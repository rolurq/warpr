//! Client for a Herdr session's JSON socket API.
//!
//! The crate turns what a session reports into a [`PresentedSession`], the state a front-end
//! renders. It owns no UI and no async runtime: calls block, so callers run them off their UI
//! thread. [`LiveSession`] is the exception: it keeps a presented state current from background
//! threads of its own.
//!
//! Every call addresses the socket of an explicit [`SessionTarget`]. Herdr's own
//! `HERDR_SOCKET_PATH` points at whichever session the launching shell lives in, so it is never
//! consulted.

mod cli;
mod compat;
#[cfg(unix)]
mod events;
#[cfg(unix)]
mod live;
#[cfg(not(unix))]
#[path = "live_unsupported.rs"]
mod live;
mod presented;
mod reconnect;
mod session;
mod sessions;
mod socket;
mod tabs;

use std::time::Duration;

pub use cli::{HERDR_BINARY_ENV, HerdrCli};
pub use compat::Refusal;
pub use live::LiveSession;
pub use presented::{
    ConnectionFailure, ConnectionStatus, PresentedSession, PresentedWorkspace, WorkspaceId,
};
pub use reconnect::ReconnectPolicy;
use serde::Deserialize;
use serde_json::json;
pub use session::{DEFAULT_SESSION, InvalidSessionName, SessionName, SessionTarget, config_dir};
pub use sessions::{ListedSession, SessionState, launch_session, list_sessions, open_session};
pub use tabs::{CellRect, HerdrTabId, PaneId, PaneLayout, PresentedHerdrTab, PresentedPane};

/// How long each `herdr status` run in the compatibility check may take; it matches the socket
/// client's timeout.
const STATUS_TIMEOUT: Duration = Duration::from_secs(10);

/// Connects to `target` and presents its current workspaces.
///
/// The `herdr` executable behind `cli` is checked first, and a session is only connected to when
/// it is compatible: otherwise nothing of the session is presented and
/// [`PresentedSession::connection`] holds the [`Refusal`].
///
/// Failure to connect is reported in [`PresentedSession::connection`] rather than as an error,
/// because a front-end shows it as part of the presented state.
pub fn present_session(cli: &HerdrCli, target: &SessionTarget) -> PresentedSession {
    if let Err(refusal) = compat::check(cli, target, STATUS_TIMEOUT) {
        return PresentedSession::refused(refusal);
    }
    match fetch_snapshot(target) {
        Ok(snapshot) => PresentedSession::from_snapshot(snapshot),
        Err(failure) => PresentedSession::failed(failure),
    }
}

#[derive(Deserialize)]
struct SnapshotResult {
    snapshot: presented::Snapshot,
}

pub(crate) fn fetch_snapshot(
    target: &SessionTarget,
) -> Result<presented::Snapshot, ConnectionFailure> {
    let result = socket::request(target.socket_path(), "session.snapshot", json!({}))?;
    let SnapshotResult { snapshot } = serde_json::from_value(result)
        .map_err(|error| ConnectionFailure::Protocol(error.to_string()))?;
    Ok(snapshot)
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
