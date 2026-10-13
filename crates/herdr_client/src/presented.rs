use std::path::PathBuf;

use serde::Deserialize;

/// Herdr's identifier for a workspace, stable for the life of the workspace.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize)]
#[serde(transparent)]
pub struct WorkspaceId(String);

impl WorkspaceId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for WorkspaceId {
    fn from(id: &str) -> Self {
        Self(id.to_owned())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct PresentedWorkspace {
    #[serde(rename = "workspace_id")]
    pub id: WorkspaceId,
    pub label: String,
}

/// Why a session could not be presented.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConnectionFailure {
    #[error("could not reach the Herdr session socket at {}: {message}", path.display())]
    SocketUnreachable { path: PathBuf, message: String },
    #[error("Herdr sent a response this client does not understand: {0}")]
    Protocol(String),
    #[error("Herdr rejected the request ({code}): {message}")]
    Rejected { code: String, message: String },
    #[error("Herdr sessions are only reachable over unix sockets, which this platform lacks")]
    UnsupportedPlatform,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionStatus {
    Connecting,
    Connected,
    Failed(ConnectionFailure),
}

/// The state of a Herdr session as a front-end presents it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresentedSession {
    pub connection: ConnectionStatus,
    /// In the order Herdr lists them.
    pub workspaces: Vec<PresentedWorkspace>,
    pub focused_workspace: Option<WorkspaceId>,
}

impl PresentedSession {
    pub fn connecting() -> Self {
        Self::without_workspaces(ConnectionStatus::Connecting)
    }

    pub(crate) fn failed(failure: ConnectionFailure) -> Self {
        Self::without_workspaces(ConnectionStatus::Failed(failure))
    }

    fn without_workspaces(connection: ConnectionStatus) -> Self {
        Self {
            connection,
            workspaces: Vec::new(),
            focused_workspace: None,
        }
    }

    pub(crate) fn from_snapshot(snapshot: Snapshot) -> Self {
        Self {
            connection: ConnectionStatus::Connected,
            workspaces: snapshot.workspaces,
            focused_workspace: snapshot.focused_workspace_id,
        }
    }

    /// Position of the focused workspace in [`Self::workspaces`].
    pub fn focused_workspace_index(&self) -> Option<usize> {
        let focused = self.focused_workspace.as_ref()?;
        self.workspaces
            .iter()
            .position(|workspace| &workspace.id == focused)
    }
}

/// The part of `session.snapshot` this client presents; Herdr sends far more.
#[derive(Deserialize)]
pub(crate) struct Snapshot {
    #[serde(default)]
    focused_workspace_id: Option<WorkspaceId>,
    workspaces: Vec<PresentedWorkspace>,
}
