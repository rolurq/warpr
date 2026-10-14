//! warpr's connection to a Herdr session.
//!
//! Herdr owns every workspace, tab and pane; this module only holds the state the UI presents.

mod pane_frames;
mod workspace_view;

use herdr_client::{
    ConnectionFailure, ConnectionStatus, PresentedSession, SessionTarget, present_session,
};
use warpui::{Entity, ModelContext, SingletonEntity};
pub use workspace_view::HerdrWorkspaceView;

/// Names the Herdr session to present. Defaults to Herdr's `default` session.
const SESSION_ENV: &str = "WARPR_HERDR_SESSION";

pub(crate) struct HerdrSessionModel {
    presented: PresentedSession,
}

pub(crate) enum HerdrSessionEvent {
    Presented,
}

impl HerdrSessionModel {
    /// Starts connecting to the session named by [`SESSION_ENV`] in the background.
    pub(crate) fn new(ctx: &mut ModelContext<Self>) -> Self {
        let name =
            std::env::var(SESSION_ENV).unwrap_or_else(|_| herdr_client::DEFAULT_SESSION.to_owned());
        match SessionTarget::for_current_user(&name) {
            Ok(target) => Self::connect(target, ctx),
            Err(failure) => Self::failed(failure),
        }
    }

    /// Starts connecting to `target` in the background.
    pub(crate) fn connect(target: SessionTarget, ctx: &mut ModelContext<Self>) -> Self {
        ctx.spawn(
            async move { present_session(&target) },
            |me, presented, ctx| me.set_presented(presented, ctx),
        );
        Self {
            presented: PresentedSession::connecting(),
        }
    }

    fn failed(failure: ConnectionFailure) -> Self {
        log::error!("Could not present the Herdr session: {failure}");
        Self {
            presented: PresentedSession::failed(failure),
        }
    }

    pub(crate) fn presented(&self) -> &PresentedSession {
        &self.presented
    }

    fn set_presented(&mut self, presented: PresentedSession, ctx: &mut ModelContext<Self>) {
        if let ConnectionStatus::Failed(failure) = &presented.connection {
            log::error!("Could not present the Herdr session: {failure}");
        }
        self.presented = presented;
        ctx.emit(HerdrSessionEvent::Presented);
    }
}

impl Entity for HerdrSessionModel {
    type Event = HerdrSessionEvent;
}

impl SingletonEntity for HerdrSessionModel {}
