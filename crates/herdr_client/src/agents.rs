use serde::Deserialize;
use serde_json::json;

use crate::compat::Refusal;
use crate::{
    ConnectionFailure, ConnectionStatus, HerdrTabId, PaneId, PresentedHerdrTab, PresentedSession,
    PresentedWorkspace, SessionTarget, WorkspaceId, socket,
};

/// Herdr's lifecycle classification of an agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentState {
    Idle,
    Working,
    Blocked,
    Done,
    Unknown,
}

impl AgentState {
    /// The name Herdr gives the state.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Working => "working",
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::Unknown => "unknown",
        }
    }
}

/// An agent Herdr recognizes in one of its panes.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct PresentedAgent {
    #[serde(rename = "pane_id")]
    pub pane: PaneId,
    #[serde(rename = "workspace_id")]
    pub workspace: WorkspaceId,
    #[serde(rename = "tab_id")]
    pub herdr_tab: HerdrTabId,
    /// What Herdr recognized, such as `claude`. Herdr may not know it.
    #[serde(rename = "agent", default)]
    pub kind: Option<String>,
    /// The agent's live name. Most agents have none.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(rename = "agent_status")]
    pub state: AgentState,
}

/// Where an agent sits in a presented session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentLocation<'a> {
    pub workspace: &'a PresentedWorkspace,
    pub herdr_tab: &'a PresentedHerdrTab,
}

/// What a front-end shows to take the user to an agent's pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentJump {
    pub workspace: WorkspaceId,
    pub herdr_tab: HerdrTabId,
    pub pane: PaneId,
}

impl PresentedAgent {
    /// The workspace and Herdr tab of this agent in `session`, or `None` while `session` does not
    /// show them, as when the agent appeared after the session was last read.
    pub fn location<'a>(&self, session: &'a PresentedSession) -> Option<AgentLocation<'a>> {
        let workspace = session.workspace(&self.workspace)?;
        let herdr_tab = workspace
            .herdr_tabs
            .iter()
            .find(|tab| tab.id == self.herdr_tab)?;
        Some(AgentLocation {
            workspace,
            herdr_tab,
        })
    }

    /// Where to take the user to reach this agent's pane, or `None` while `session` does not show
    /// that pane. Resolving it asks nothing of Herdr: it never moves Herdr's focus.
    pub fn jump_target(&self, session: &PresentedSession) -> Option<AgentJump> {
        let location = self.location(session)?;
        let shows_pane = location
            .herdr_tab
            .layout
            .as_ref()
            .is_none_or(|layout| layout.panes.iter().any(|pane| pane.id == self.pane));
        shows_pane.then(|| AgentJump {
            workspace: self.workspace.clone(),
            herdr_tab: self.herdr_tab.clone(),
            pane: self.pane.clone(),
        })
    }
}

/// The agents of a Herdr session as a front-end presents them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresentedAgents {
    pub connection: ConnectionStatus,
    /// In the order Herdr lists them.
    pub agents: Vec<PresentedAgent>,
}

impl PresentedAgents {
    pub fn connecting() -> Self {
        Self::without_agents(ConnectionStatus::Connecting)
    }

    pub fn failed(failure: ConnectionFailure) -> Self {
        Self::without_agents(ConnectionStatus::Failed(failure))
    }

    pub(crate) fn refused(refusal: Refusal) -> Self {
        Self::without_agents(ConnectionStatus::Refused(refusal))
    }

    pub(crate) fn connected(agents: Vec<PresentedAgent>) -> Self {
        Self {
            connection: ConnectionStatus::Connected,
            agents,
        }
    }

    fn without_agents(connection: ConnectionStatus) -> Self {
        Self {
            connection,
            agents: Vec::new(),
        }
    }

    /// This state with the connection reported as `connection`, keeping what was last presented.
    pub(crate) fn with_connection(self, connection: ConnectionStatus) -> Self {
        Self { connection, ..self }
    }

    pub fn agent(&self, pane: &PaneId) -> Option<&PresentedAgent> {
        self.agents.iter().find(|agent| &agent.pane == pane)
    }
}

#[derive(Deserialize)]
struct AgentList {
    agents: Vec<PresentedAgent>,
}

#[derive(Deserialize)]
struct PaneList {
    panes: Vec<PaneRef>,
}

#[derive(Deserialize)]
struct PaneRef {
    pane_id: PaneId,
}

/// Every agent Herdr recognizes right now.
pub(crate) fn fetch_agents(
    target: &SessionTarget,
) -> Result<Vec<PresentedAgent>, ConnectionFailure> {
    let result = socket::request(target.socket_path(), "agent.list", json!({}))?;
    let AgentList { agents } = serde_json::from_value(result)
        .map_err(|error| ConnectionFailure::Protocol(error.to_string()))?;
    Ok(agents)
}

/// Every pane Herdr has right now, agent or not: any of them may come to hold an agent.
pub(crate) fn fetch_pane_ids(target: &SessionTarget) -> Result<Vec<PaneId>, ConnectionFailure> {
    let result = socket::request(target.socket_path(), "pane.list", json!({}))?;
    let PaneList { panes } = serde_json::from_value(result)
        .map_err(|error| ConnectionFailure::Protocol(error.to_string()))?;
    Ok(panes.into_iter().map(|pane| pane.pane_id).collect())
}
