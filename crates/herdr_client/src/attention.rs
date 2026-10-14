use crate::agents::PresentedAgents;
use crate::desktop::{DesktopAction, DesktopNotifications};
use crate::notifications::{AgentNotifications, NotificationChange};
use crate::viewing::{ViewedPanes, ViewerId};
use crate::{PaneId, WorkspaceId};

/// What a front-end has to do after something changed in what the user can see or in the agents.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AttentionChanges {
    /// Changes to the front-end's own notifications, such as a toast and a mailbox item.
    pub notifications: Vec<NotificationChange>,
    /// Changes to the operating system's notification centre.
    pub desktop: Vec<DesktopAction>,
}

impl AttentionChanges {
    pub fn is_empty(&self) -> bool {
        self.notifications.is_empty() && self.desktop.is_empty()
    }
}

/// Decides how a front-end tells the user about agents, from what its windows show and which of
/// them has focus.
///
/// A pane counts as seen when the focused window has the pane's workspace selected and shows the
/// pane. Seen state is held per window, so several windows never overwrite each other.
/// Herdr is never asked to mark anything seen, because that would move its focus.
#[derive(Debug, Default)]
pub struct AgentAttention {
    notifications: AgentNotifications,
    desktop: DesktopNotifications,
    viewed: ViewedPanes,
    focused: Option<ViewerId>,
}

impl AgentAttention {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes the agents as presented now. See [`AgentNotifications::observe`].
    pub fn observe(&mut self, presented: &PresentedAgents) -> AttentionChanges {
        let notifications = self.notifications.observe(presented);
        let desktop = self.desktop.apply(&notifications, self.focused.is_some());
        AttentionChanges {
            notifications,
            desktop,
        }
    }

    /// Records the panes `viewer`'s view of `workspace` shows.
    pub fn show(
        &mut self,
        viewer: ViewerId,
        workspace: WorkspaceId,
        panes: impl IntoIterator<Item = PaneId>,
    ) -> AttentionChanges {
        self.viewed.show(viewer, workspace, panes);
        self.reconcile()
    }

    /// Records which workspace `viewer` has selected, or none.
    pub fn select(&mut self, viewer: ViewerId, workspace: Option<WorkspaceId>) -> AttentionChanges {
        self.viewed.select(viewer, workspace);
        self.reconcile()
    }

    /// Records which viewer has the user's attention, or that none has because the front-end is
    /// not the focused application.
    pub fn focus(&mut self, focused: Option<ViewerId>) -> AttentionChanges {
        self.focused = focused;
        self.reconcile()
    }

    /// Forgets everything announced and shown, as when the session is no longer followed.
    pub fn reset(&mut self) -> AttentionChanges {
        self.notifications = AgentNotifications::new();
        self.viewed.clear();
        AttentionChanges {
            notifications: Vec::new(),
            desktop: self.desktop.withdraw_all(),
        }
    }

    fn reconcile(&mut self) -> AttentionChanges {
        let viewed = self.viewed.viewed(self.focused);
        let notifications = self.notifications.set_viewed(viewed.iter().cloned());
        let application_focused = self.focused.is_some();
        let mut desktop = self.desktop.apply(&notifications, application_focused);
        desktop.extend(self.desktop.attention_changed(application_focused, &viewed));
        AttentionChanges {
            notifications,
            desktop,
        }
    }
}

#[cfg(all(test, unix))]
#[path = "attention_tests.rs"]
mod tests;
