use serde_json::json;

use crate::{ConnectionFailure, SessionTarget, WorkspaceId, socket};

/// Asks the session of `target` to create a workspace and focus it.
///
/// Nothing is returned to present: the workspace reaches a front-end through the session's own
/// state, like any other change Herdr makes. A refused request is [`ConnectionFailure::Rejected`].
pub fn create_workspace(target: &SessionTarget) -> Result<(), ConnectionFailure> {
    socket::request(
        target.socket_path(),
        "workspace.create",
        json!({ "focus": true }),
    )
    .map(drop)
}

/// Asks the session of `target` to create a Herdr tab in `workspace` and focus it. As with
/// [`create_workspace`], the tab is only presented once the session reports it.
pub fn create_herdr_tab(
    target: &SessionTarget,
    workspace: &WorkspaceId,
) -> Result<(), ConnectionFailure> {
    socket::request(
        target.socket_path(),
        "tab.create",
        json!({ "workspace_id": workspace.as_str(), "focus": true }),
    )
    .map(drop)
}

#[cfg(test)]
#[path = "create_tests.rs"]
mod tests;
