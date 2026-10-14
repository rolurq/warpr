use std::collections::{HashMap, HashSet};

use crate::agents::{AgentState, PresentedAgent, PresentedAgents};
use crate::{ConnectionStatus, PaneId};

/// Why a front-end tells the user about an agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NotificationKind {
    /// The agent is waiting on an approval or a question.
    Blocked,
    /// The agent finished work the user has not looked at.
    Finished,
}

impl NotificationKind {
    /// The agent state that keeps a notification of this kind relevant.
    fn state(self) -> AgentState {
        match self {
            Self::Blocked => AgentState::Blocked,
            Self::Finished => AgentState::Done,
        }
    }
}

/// What a front-end shows to tell the user about one agent, for as long as it stays relevant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentNotification {
    pub kind: NotificationKind,
    /// The agent as Herdr presented it when the notification was raised: its name, kind and where
    /// it runs. Resolve the location with [`PresentedAgent::location`] and
    /// [`PresentedAgent::jump_target`] against the current session.
    pub agent: PresentedAgent,
}

/// A change to the notifications a front-end shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotificationChange {
    Raised(AgentNotification),
    /// The notification of `kind` for the agent in `pane` no longer applies.
    Retired {
        pane: PaneId,
        kind: NotificationKind,
    },
}

/// Decides which agents warrant a notification as their states change.
///
/// It sees only what a connected session presents, so a state that predates the first connection
/// is never announced and an outage changes nothing: the first read after it is compared with the
/// last one before it. Whether the user has seen an agent's pane is the front-end's to report
/// through [`Self::set_viewed`]; Herdr is never asked to mark anything seen, because that would
/// move its focus.
#[derive(Debug, Default)]
pub struct AgentNotifications {
    /// Each agent's state in the last connected read. `None` until there has been one.
    known: Option<HashMap<PaneId, AgentState>>,
    raised: HashMap<PaneId, NotificationKind>,
    viewed: HashSet<PaneId>,
}

impl AgentNotifications {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes the agents as presented now and returns what changed in the notifications.
    pub fn observe(&mut self, presented: &PresentedAgents) -> Vec<NotificationChange> {
        if presented.connection != ConnectionStatus::Connected {
            return Vec::new();
        }
        let previous = self.known.replace(
            presented
                .agents
                .iter()
                .map(|agent| (agent.pane.clone(), agent.state))
                .collect(),
        );

        let mut changes = Vec::new();
        self.raised.retain(|pane, kind| {
            let relevant = presented
                .agent(pane)
                .is_some_and(|agent| agent.state == kind.state());
            if !relevant {
                changes.push(NotificationChange::Retired {
                    pane: pane.clone(),
                    kind: *kind,
                });
            }
            relevant
        });

        let Some(previous) = previous else {
            return changes;
        };
        for agent in &presented.agents {
            let before = previous.get(&agent.pane).copied();
            let kind = match (before, agent.state) {
                (Some(AgentState::Blocked), AgentState::Blocked) => None,
                (_, AgentState::Blocked) => Some(NotificationKind::Blocked),
                (Some(AgentState::Working), AgentState::Done)
                    if !self.viewed.contains(&agent.pane) =>
                {
                    Some(NotificationKind::Finished)
                }
                (
                    _,
                    AgentState::Idle | AgentState::Working | AgentState::Done | AgentState::Unknown,
                ) => None,
            };
            if let Some(kind) = kind {
                self.raised.insert(agent.pane.clone(), kind);
                changes.push(NotificationChange::Raised(AgentNotification {
                    kind,
                    agent: agent.clone(),
                }));
            }
        }
        changes
    }

    /// Records which panes the user is looking at, replacing what was recorded before. Looking at
    /// an agent's pane retires its finished notification, and an agent that finishes while its
    /// pane is in view raises none.
    pub fn set_viewed(
        &mut self,
        panes: impl IntoIterator<Item = PaneId>,
    ) -> Vec<NotificationChange> {
        self.viewed = panes.into_iter().collect();
        let mut changes = Vec::new();
        self.raised.retain(|pane, kind| {
            let seen = *kind == NotificationKind::Finished && self.viewed.contains(pane);
            if seen {
                changes.push(NotificationChange::Retired {
                    pane: pane.clone(),
                    kind: *kind,
                });
            }
            !seen
        });
        changes
    }
}

#[cfg(all(test, unix))]
#[path = "notifications_tests.rs"]
mod tests;
