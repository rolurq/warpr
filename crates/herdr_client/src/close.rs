use serde_json::json;

use crate::{ConnectionFailure, PaneId, SessionTarget, socket};

/// Asks the session of `target` to close `pane`.
///
/// Nothing is returned to present: the pane leaves a front-end through the session's own state,
/// like any other change Herdr makes. Herdr closes the Herdr tab, and the workspace, along with
/// their last pane. A refused request is [`ConnectionFailure::Rejected`].
pub fn close_pane(target: &SessionTarget, pane: &PaneId) -> Result<(), ConnectionFailure> {
    socket::request(
        target.socket_path(),
        "pane.close",
        json!({ "pane_id": pane.as_str() }),
    )
    .map(drop)
}

#[cfg(test)]
#[path = "close_tests.rs"]
mod tests;
