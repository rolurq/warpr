use std::collections::{HashMap, HashSet};

use crate::PaneId;
use crate::notifications::{AgentNotification, NotificationChange, NotificationKind};

/// What a front-end does with the operating system's notification centre.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesktopAction {
    /// Shows the notification, replacing the one already shown for the same pane.
    Raise(AgentNotification),
    /// Removes the notification shown for the agent in this pane.
    Withdraw(PaneId),
}

/// Decides when the operating system is told about an agent, and when it is told to forget.
///
/// An agent is announced there only while the front-end is not the focused application: with
/// focus, the front-end's own notifications are enough. It never consults Herdr's notification
/// settings. Herdr hands its own notifications (toasts, sounds) only to an attached Herdr
/// interface, and nothing a client can ask tells whether one is attached, so there is no case in
/// which suppressing here is known to be safe, and asking Herdr to show something would add a
/// notification of Herdr's own on top of the front-end's.
#[derive(Debug, Default)]
pub struct DesktopNotifications {
    delivered: HashMap<PaneId, NotificationKind>,
}

impl DesktopNotifications {
    pub fn new() -> Self {
        Self::default()
    }

    /// What to do about `changes` to the front-end's own notifications, given whether the
    /// front-end is the focused application.
    pub fn apply(
        &mut self,
        changes: &[NotificationChange],
        application_focused: bool,
    ) -> Vec<DesktopAction> {
        let mut actions = Vec::new();
        for change in changes {
            match change {
                NotificationChange::Raised(notification) if !application_focused => {
                    self.delivered
                        .insert(notification.agent.pane.clone(), notification.kind);
                    actions.push(DesktopAction::Raise(notification.clone()));
                }
                NotificationChange::Raised(_) => {}
                NotificationChange::Retired { pane, kind } => {
                    if self.delivered.get(pane) == Some(kind) {
                        self.delivered.remove(pane);
                        actions.push(DesktopAction::Withdraw(pane.clone()));
                    }
                }
            }
        }
        actions
    }

    /// What to do once the user is back looking at the front-end: whatever was announced about a
    /// pane now in `viewed` is withdrawn. Nothing is withdrawn while the front-end is not the
    /// focused application.
    pub fn attention_changed(
        &mut self,
        application_focused: bool,
        viewed: &HashSet<PaneId>,
    ) -> Vec<DesktopAction> {
        if !application_focused {
            return Vec::new();
        }
        let mut actions = Vec::new();
        self.delivered.retain(|pane, _| {
            let seen = viewed.contains(pane);
            if seen {
                actions.push(DesktopAction::Withdraw(pane.clone()));
            }
            !seen
        });
        actions
    }

    /// Withdraws everything announced, as when the session announced about is no longer followed.
    pub fn withdraw_all(&mut self) -> Vec<DesktopAction> {
        self.delivered
            .drain()
            .map(|(pane, _)| DesktopAction::Withdraw(pane))
            .collect()
    }
}
