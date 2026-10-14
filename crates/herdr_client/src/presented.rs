use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;

use crate::compat::Refusal;
use crate::tabs::{HerdrTabId, PresentedHerdrTab, TabInfo, TabLayout};

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresentedWorkspace {
    pub id: WorkspaceId,
    pub label: String,
    /// In the order Herdr lists them.
    pub herdr_tabs: Vec<PresentedHerdrTab>,
    /// The Herdr tab Herdr has focused in this workspace.
    pub active_herdr_tab: Option<HerdrTabId>,
}

impl PresentedWorkspace {
    /// The Herdr tab to show given the user's own pick, which is view state and never reaches
    /// Herdr. Falls back to Herdr's active tab, then the first, when there is no pick or the
    /// picked tab is gone.
    pub fn selected_tab(&self, picked: Option<&HerdrTabId>) -> Option<&PresentedHerdrTab> {
        let with_id = |id: &HerdrTabId| self.herdr_tabs.iter().find(|tab| &tab.id == id);
        picked
            .and_then(with_id)
            .or_else(|| self.active_herdr_tab.as_ref().and_then(with_id))
            .or_else(|| self.herdr_tabs.first())
    }
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
    #[error("could not locate the Herdr session {name:?}: {reason}")]
    SessionUnresolvable { name: String, reason: String },
    #[error("could not list the Herdr sessions: {reason}")]
    SessionListUnavailable { reason: String },
    #[error("could not start the Herdr session {name:?}: {reason}")]
    ServerStartFailed { name: String, reason: String },
    #[error("Herdr sessions are only reachable over unix sockets, which this platform lacks")]
    UnsupportedPlatform,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionStatus {
    Connecting,
    Connected,
    Failed(ConnectionFailure),
    /// The `herdr` executable is missing or unusable, or the server is incompatible with it; no
    /// workspaces are presented.
    Refused(Refusal),
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

    pub fn failed(failure: ConnectionFailure) -> Self {
        Self::without_workspaces(ConnectionStatus::Failed(failure))
    }

    pub fn refused(refusal: Refusal) -> Self {
        Self::without_workspaces(ConnectionStatus::Refused(refusal))
    }

    fn without_workspaces(connection: ConnectionStatus) -> Self {
        Self {
            connection,
            workspaces: Vec::new(),
            focused_workspace: None,
        }
    }

    pub(crate) fn from_snapshot(snapshot: Snapshot) -> Self {
        let focused_workspace = snapshot.focused_workspace_id.clone();
        Self {
            connection: ConnectionStatus::Connected,
            workspaces: snapshot.presented_workspaces(),
            focused_workspace,
        }
    }

    pub fn workspace(&self, id: &WorkspaceId) -> Option<&PresentedWorkspace> {
        self.workspaces.iter().find(|workspace| &workspace.id == id)
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
    workspaces: Vec<WorkspaceInfo>,
    tabs: Vec<TabInfo>,
    layouts: Vec<TabLayout>,
}

#[derive(Deserialize)]
struct WorkspaceInfo {
    workspace_id: WorkspaceId,
    label: String,
    #[serde(default)]
    active_tab_id: Option<HerdrTabId>,
}

impl Snapshot {
    /// Groups the flat tab and layout lists under their workspaces, keeping Herdr's order.
    fn presented_workspaces(self) -> Vec<PresentedWorkspace> {
        let Self {
            workspaces,
            tabs,
            mut layouts,
            ..
        } = self;
        let mut tabs_by_workspace: HashMap<WorkspaceId, Vec<PresentedHerdrTab>> = HashMap::new();
        for tab in tabs {
            let layout = layouts
                .iter()
                .position(|candidate| candidate.tab_id == tab.tab_id)
                .map(|index| layouts.swap_remove(index).layout);
            tabs_by_workspace
                .entry(tab.workspace_id)
                .or_default()
                .push(PresentedHerdrTab {
                    id: tab.tab_id,
                    label: tab.label,
                    layout,
                });
        }
        workspaces
            .into_iter()
            .map(|workspace| PresentedWorkspace {
                herdr_tabs: tabs_by_workspace
                    .remove(&workspace.workspace_id)
                    .unwrap_or_default(),
                id: workspace.workspace_id,
                label: workspace.label,
                active_herdr_tab: workspace.active_tab_id,
            })
            .collect()
    }
}
