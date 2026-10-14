//! warpr's connection to a Herdr session.
//!
//! Herdr owns every workspace, tab and pane; this module only holds the state the UI presents.

mod pane_frames;
mod session_picker;
mod sessions;
mod workspace_view;

use herdr_client::{ConnectionFailure, ConnectionStatus, HerdrCli, PresentedSession};
pub use session_picker::HerdrSessionPicker;
use sessions::SessionSelection;
use warpui::{Entity, ModelContext, SingletonEntity};
pub use workspace_view::HerdrWorkspaceView;

/// Names the Herdr session to present. Defaults to Herdr's `default` session.
const SESSION_ENV: &str = "WARPR_HERDR_SESSION";

pub(crate) struct HerdrSessionModel {
    presented: PresentedSession,
    sessions: SessionSelection,
}

pub(crate) enum HerdrSessionEvent {
    Presented,
    /// Herdr's session listing was read again.
    SessionsListed,
}

impl HerdrSessionModel {
    /// Starts connecting to the session named by [`SESSION_ENV`] in the background, starting its
    /// server first when Herdr reports it stopped.
    pub(crate) fn new(ctx: &mut ModelContext<Self>) -> Self {
        let name =
            std::env::var(SESSION_ENV).unwrap_or_else(|_| herdr_client::DEFAULT_SESSION.to_owned());
        Self::launch(HerdrCli::from_env(), herdr_client::config_dir(), &name, ctx)
    }

    /// Starts connecting to `target` in the background. Unlike [`Self::new`] it never starts a
    /// server, so the session must already be running.
    #[cfg(test)]
    pub(crate) fn connect(
        target: herdr_client::SessionTarget,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.spawn(
            async move { herdr_client::present_session(&target) },
            |me, presented, ctx| me.set_presented(presented, ctx),
        );
        Self {
            presented: PresentedSession::connecting(),
            sessions: SessionSelection::new(HerdrCli::from_env(), herdr_client::config_dir()),
        }
    }

    fn failed(failure: ConnectionFailure, sessions: SessionSelection) -> Self {
        log::error!("Could not present the Herdr session: {failure}");
        Self {
            presented: PresentedSession::failed(failure),
            sessions,
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
