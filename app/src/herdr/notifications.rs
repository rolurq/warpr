//! Telling the user when a Herdr agent needs them or has finished, through Warp's agent
//! notifications.

use herdr_client::{
    AgentNotification, NotificationChange, NotificationKind, PaneId, PresentedAgents,
    PresentedSession,
};
use warpui::{ModelContext, SingletonEntity as _};

use super::HerdrSessionModel;
use crate::ai::agent_management::AgentNotificationsModel;
use crate::ai::agent_management::notifications::{NotificationCategory, NotificationItem};
use crate::terminal::CLIAgent;

const NO_KIND: &str = "unknown kind";

/// The Warp notification for `notification`, naming the agent and where it runs in `session`.
fn item_for(notification: &AgentNotification, session: &PresentedSession) -> NotificationItem {
    let agent = &notification.agent;
    let title = agent
        .name
        .as_deref()
        .or(agent.kind.as_deref())
        .unwrap_or(agent.pane.as_str())
        .to_owned();
    let (workspace, herdr_tab) = match agent.location(session) {
        Some(location) => (
            location.workspace.label.clone(),
            location.herdr_tab.label.clone(),
        ),
        None => (
            agent.workspace.as_str().to_owned(),
            agent.herdr_tab.as_str().to_owned(),
        ),
    };
    let (category, what_happened) = match notification.kind {
        NotificationKind::Blocked => (NotificationCategory::Request, "Waiting for you"),
        NotificationKind::Finished => (NotificationCategory::Complete, "Finished"),
    };
    let kind = agent.kind.as_deref().unwrap_or(NO_KIND);
    let message = format!(
        "{what_happened} · {kind} · {workspace} › {herdr_tab} › {}",
        agent.pane.as_str()
    );
    NotificationItem::for_herdr_agent(
        title,
        message,
        category,
        cli_agent(agent.kind.as_deref()),
        agent.pane.clone(),
    )
}

/// The Warp agent with the name Herdr recognized, which picks the notification's icon.
fn cli_agent(kind: Option<&str>) -> CLIAgent {
    kind.and_then(|kind| {
        enum_iterator::all::<CLIAgent>()
            .find(|agent| agent.telemetry_name().eq_ignore_ascii_case(kind))
    })
    .unwrap_or(CLIAgent::Unknown)
}

impl HerdrSessionModel {
    /// Raises and retires notifications for the agents as they are presented now.
    pub(super) fn notify_about_agents(
        &mut self,
        presented: &PresentedAgents,
        ctx: &mut ModelContext<Self>,
    ) {
        let changes = self.agents.notifications.observe(presented);
        self.apply_notification_changes(changes, ctx);
    }

    /// Records which panes the user is looking at in warpr.
    pub(crate) fn set_viewed_panes(
        &mut self,
        panes: impl IntoIterator<Item = PaneId>,
        ctx: &mut ModelContext<Self>,
    ) {
        let changes = self.agents.notifications.set_viewed(panes);
        self.apply_notification_changes(changes, ctx);
    }

    /// Forgets what was announced about the agents of a session no longer followed.
    pub(super) fn withdraw_agent_notifications(&mut self, ctx: &mut ModelContext<Self>) {
        self.agents.notifications = Default::default();
        if ctx.has_singleton_model::<AgentNotificationsModel>() {
            AgentNotificationsModel::handle(ctx).update(ctx, |model, ctx| {
                model.remove_herdr_agent_notifications(ctx);
            });
        }
    }

    fn apply_notification_changes(
        &mut self,
        changes: Vec<NotificationChange>,
        ctx: &mut ModelContext<Self>,
    ) {
        if changes.is_empty() || !ctx.has_singleton_model::<AgentNotificationsModel>() {
            return;
        }
        AgentNotificationsModel::handle(ctx).update(ctx, |model, ctx| {
            for change in changes {
                match change {
                    NotificationChange::Raised(notification) => {
                        let item = item_for(&notification, &self.presented);
                        model.add_herdr_agent_notification(item, ctx);
                    }
                    NotificationChange::Retired { pane, .. } => {
                        model.remove_herdr_agent_notification(pane, ctx);
                    }
                }
            }
        });
    }
}

#[cfg(all(test, unix))]
#[path = "notifications_tests.rs"]
mod tests;
