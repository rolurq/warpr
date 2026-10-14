//! Telling the user when a Herdr agent needs them or has finished, through Warp's agent
//! notifications and, while warpr is not the focused application, the operating system's.

use herdr_client::{
    AgentNotification, AttentionChanges, NotificationChange, NotificationKind, PaneId,
    PresentedAgents, PresentedSession, ViewerId, WorkspaceId,
};
use warpui::windowing::state::ApplicationStage;
use warpui::windowing::{StateEvent, WindowManager};
use warpui::{ModelContext, SingletonEntity as _, WindowId};

use super::HerdrSessionModel;
use crate::ai::agent_management::AgentNotificationsModel;
use crate::ai::agent_management::notifications::{NotificationCategory, NotificationItem};
use crate::terminal::CLIAgent;

const NO_KIND: &str = "unknown kind";

/// The viewer that stands for `window` in the attention the model keeps.
pub(crate) fn viewer_of(window: WindowId) -> ViewerId {
    ViewerId(window.as_usize() as u64)
}

/// The title and message that tell the user about `notification`: the agent's name, then what
/// happened, its kind, and where it runs in `session`.
pub(super) fn summary(
    notification: &AgentNotification,
    session: &PresentedSession,
) -> (String, String) {
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
    let what_happened = match notification.kind {
        NotificationKind::Blocked => "Waiting for you",
        NotificationKind::Finished => "Finished",
    };
    let kind = agent.kind.as_deref().unwrap_or(NO_KIND);
    let message = format!(
        "{what_happened} · {kind} · {workspace} › {herdr_tab} › {}",
        agent.pane.as_str()
    );
    (title, message)
}

/// The Warp notification for `notification`, naming the agent and where it runs in `session`.
fn item_for(notification: &AgentNotification, session: &PresentedSession) -> NotificationItem {
    let (title, message) = summary(notification, session);
    let category = match notification.kind {
        NotificationKind::Blocked => NotificationCategory::Request,
        NotificationKind::Finished => NotificationCategory::Complete,
    };
    NotificationItem::for_herdr_agent(
        title,
        message,
        category,
        cli_agent(notification.agent.kind.as_deref()),
        notification.agent.pane.clone(),
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

/// The window with the user's attention: none unless warpr is the active application.
fn focused_window(stage: ApplicationStage, active_window: Option<WindowId>) -> Option<WindowId> {
    (stage == ApplicationStage::Active)
        .then_some(active_window)
        .flatten()
}

impl HerdrSessionModel {
    /// Follows which window has the user's attention, so that agents are only announced to the
    /// operating system while no window has it.
    pub(super) fn follow_window_focus(&mut self, ctx: &mut ModelContext<Self>) {
        ctx.subscribe_to_model(
            &WindowManager::handle(ctx),
            |me, _, event, ctx| match event {
                StateEvent::ValueChanged { current, previous } => {
                    if current.active_window != previous.active_window
                        || current.stage != previous.stage
                    {
                        me.sync_window_focus(
                            focused_window(current.stage, current.active_window),
                            ctx,
                        );
                    }
                }
            },
        );
        let focused = focused_window(ctx.windows().stage(), ctx.windows().active_window());
        self.sync_window_focus(focused, ctx);
    }

    fn sync_window_focus(&mut self, focused: Option<WindowId>, ctx: &mut ModelContext<Self>) {
        let changes = self.agents.attention.focus(focused.map(viewer_of));
        self.apply_attention_changes(changes, ctx);
    }

    /// Raises and retires notifications for the agents as they are presented now.
    pub(super) fn notify_about_agents(
        &mut self,
        presented: &PresentedAgents,
        ctx: &mut ModelContext<Self>,
    ) {
        let changes = self.agents.attention.observe(presented);
        self.apply_attention_changes(changes, ctx);
    }

    /// Records the panes of `workspace` that the view in `window` shows. Only warpr's own notion:
    /// Herdr is not told.
    pub(crate) fn show_panes(
        &mut self,
        window: WindowId,
        workspace: WorkspaceId,
        panes: impl IntoIterator<Item = PaneId>,
        ctx: &mut ModelContext<Self>,
    ) {
        let changes = self
            .agents
            .attention
            .show(viewer_of(window), workspace, panes);
        self.apply_attention_changes(changes, ctx);
    }

    /// Records which workspace `window` has selected, or none when it shows something else.
    pub(crate) fn select_workspace(
        &mut self,
        window: WindowId,
        workspace: Option<WorkspaceId>,
        ctx: &mut ModelContext<Self>,
    ) {
        let changes = self.agents.attention.select(viewer_of(window), workspace);
        self.apply_attention_changes(changes, ctx);
    }

    /// Forgets what was announced about the agents of a session no longer followed.
    pub(super) fn withdraw_agent_notifications(&mut self, ctx: &mut ModelContext<Self>) {
        let changes = self.agents.attention.reset();
        self.apply_desktop_actions(changes.desktop, ctx);
        if ctx.has_singleton_model::<AgentNotificationsModel>() {
            AgentNotificationsModel::handle(ctx).update(ctx, |model, ctx| {
                model.remove_herdr_agent_notifications(ctx);
            });
        }
    }

    fn apply_attention_changes(&mut self, changes: AttentionChanges, ctx: &mut ModelContext<Self>) {
        if changes.is_empty() {
            return;
        }
        self.apply_notification_changes(changes.notifications, ctx);
        self.apply_desktop_actions(changes.desktop, ctx);
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
