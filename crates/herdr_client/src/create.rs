use std::path::PathBuf;

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

/// A linked worktree to create, as `herdr worktree create` takes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewWorktree {
    /// The repository to create the worktree in: its main checkout.
    pub repo: PathBuf,
    /// The branch the worktree checks out, created when it does not exist.
    pub branch: String,
    /// The ref a new branch starts from; Herdr chooses when `None`.
    pub base: Option<String>,
    /// Where the worktree is checked out; Herdr chooses when `None`.
    pub path: Option<PathBuf>,
    /// The new workspace's label; Herdr chooses when `None`.
    pub label: Option<String>,
}

/// Asks the session of `target` to create `worktree` and open it as a focused workspace. As with
/// [`create_workspace`], the workspace is only presented once the session reports it.
pub fn create_worktree(
    target: &SessionTarget,
    worktree: &NewWorktree,
) -> Result<(), ConnectionFailure> {
    let mut params = json!({
        "cwd": worktree.repo,
        "branch": worktree.branch,
        "focus": true,
    });
    if let Some(base) = &worktree.base {
        params["base"] = json!(base);
    }
    if let Some(path) = &worktree.path {
        params["path"] = json!(path);
    }
    if let Some(label) = &worktree.label {
        params["label"] = json!(label);
    }
    socket::request(target.socket_path(), "worktree.create", params).map(drop)
}

#[cfg(test)]
#[path = "create_tests.rs"]
mod tests;
