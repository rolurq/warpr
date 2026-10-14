//! Telling the operating system about Herdr agents.
//!
//! Unlike Warp's notifications about terminal blocks, these need no terminal view, play no sound of
//! their own and record no telemetry. A notification the OS refuses to show is dropped quietly,
//! because the user still has the in-app notification for it.

use herdr_client::{AgentNotification, DesktopAction, PaneId};
use warpui::notification::{RequestPermissionsOutcome, UserNotification};
use warpui::{AppContext, ModelContext, SingletonEntity as _, WindowId};

use super::HerdrSessionModel;
use super::notifications::summary;
use crate::notification::NotificationContext;
use crate::terminal::session_settings::{NotificationsMode, SessionSettings};

/// Names the OS notification about the agent in `pane`, so a newer one replaces the older and
/// withdrawing it finds it.
fn identifier_for(pane: &PaneId) -> String {
    format!("herdr-agent:{}", pane.as_str())
}

/// At most `max` characters of `text`, with an ellipsis where it was cut.
fn truncated(text: String, max: usize) -> String {
    if text.chars().count() <= max {
        return text;
    }
    let mut cut: String = text.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

impl HerdrSessionModel {
    pub(super) fn apply_desktop_actions(
        &mut self,
        actions: Vec<DesktopAction>,
        ctx: &mut ModelContext<Self>,
    ) {
        for action in actions {
            match action {
                DesktopAction::Raise(notification) => self.raise_desktop(&notification, ctx),
                DesktopAction::Withdraw(pane) => {
                    ctx.withdraw_desktop_notification(&identifier_for(&pane));
                }
            }
        }
    }

    fn raise_desktop(&mut self, notification: &AgentNotification, ctx: &mut ModelContext<Self>) {
        let desktop_notifications_off = ctx.has_singleton_model::<SessionSettings>()
            && SessionSettings::as_ref(ctx).notifications.mode == NotificationsMode::Disabled;
        if desktop_notifications_off {
            return;
        }
        let Some(window_id) = ctx.window_ids().next() else {
            return;
        };
        let pane = &notification.agent.pane;
        let Ok(data) = serde_json::to_string(&NotificationContext::HerdrAgent {
            pane: pane.as_str().to_owned(),
        }) else {
            return;
        };
        let (title, message) = summary(notification, &self.presented);
        let content = UserNotification::new_with_sound(
            truncated(title, UserNotification::MAX_TITLE_LENGTH),
            truncated(message, UserNotification::MAX_BODY_LENGTH),
            Some(data),
            false,
        )
        .with_identifier(identifier_for(pane));

        if self.agents.asked_for_desktop_permission {
            send(content, window_id, ctx);
            return;
        }
        self.agents.asked_for_desktop_permission = true;
        ctx.request_desktop_notification_permissions_detached(move |outcome, ctx| match outcome {
            RequestPermissionsOutcome::Accepted => send(content, window_id, ctx),
            RequestPermissionsOutcome::PermissionsDenied => {
                log::debug!("Desktop notification permission denied; agents stay in-app");
            }
            RequestPermissionsOutcome::OtherError { error_message } => {
                log::debug!("Could not ask for desktop notification permission: {error_message}");
            }
        });
    }
}

fn send(content: UserNotification, window_id: WindowId, ctx: &mut AppContext) {
    ctx.send_desktop_notification_detached(content, window_id, |error| {
        log::debug!("Desktop notification for a Herdr agent not shown: {error:?}");
    });
}

/// Brings warpr forward and takes the user to the pane of the agent an OS notification was about.
/// Only warpr's own view changes: Herdr's focus is left alone.
pub(crate) fn show_notified_agent(pane: &str, ctx: &mut AppContext) {
    ctx.windows().activate_app();
    if ctx.has_singleton_model::<HerdrSessionModel>() {
        HerdrSessionModel::handle(ctx).update(ctx, |model, ctx| {
            model.jump_to_agent(&PaneId::from(pane), ctx);
        });
    }
}
