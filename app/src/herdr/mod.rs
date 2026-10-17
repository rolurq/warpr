//! warpr's connection to a Herdr session.
//!
//! Herdr owns every workspace, tab and pane; this module only holds the state the UI presents.

mod agent_panel;
mod agents;
mod desktop;
mod directory_git;
mod live;
mod notifications;
mod pane_chrome;
mod pane_contents;
mod pane_frames;
mod pane_grid;
mod pane_input;
mod pane_terminal;
pub(crate) mod repository_root;
mod session_picker;
mod sessions;
mod workspace_view;

pub use agent_panel::HerdrAgentPanelView;
use agents::AgentTracking;
pub(crate) use desktop::show_notified_agent;
pub use directory_git::DirectoryGitContext;
use herdr_client::{
    AgentJump, ConnectionFailure, ConnectionStatus, HerdrCli, LiveSession, PresentedSession,
};
pub use session_picker::HerdrSessionPicker;
use sessions::SessionSelection;
use warpui::{Entity, ModelContext, SingletonEntity};
pub use workspace_view::{HerdrBarTab, HerdrPaneRow, HerdrWorkspaceView, HerdrWorkspaceViewAction};

/// Names the Herdr session to present. Defaults to Herdr's `default` session.
const SESSION_ENV: &str = "WARPR_HERDR_SESSION";

pub(crate) struct HerdrSessionModel {
    presented: PresentedSession,
    sessions: SessionSelection,
    /// Keeps the presented session current for as long as it lives.
    live_session: Option<LiveSession>,
    agents: AgentTracking,
}

pub(crate) enum HerdrSessionEvent {
    Presented,
    /// Herdr's session listing was read again.
    SessionsListed,
    /// The session's agents, or the connection they are read over, changed.
    AgentsPresented,
    /// The user asked to go to an agent's pane. Windows show it locally; Herdr is not told.
    JumpedToAgent(AgentJump),
}

impl HerdrSessionModel {
    /// Starts connecting to the session named by [`SESSION_ENV`] in the background, starting its
    /// server first when Herdr reports it stopped.
    pub(crate) fn new(ctx: &mut ModelContext<Self>) -> Self {
        let name =
            std::env::var(SESSION_ENV).unwrap_or_else(|_| herdr_client::DEFAULT_SESSION.to_owned());
        Self::launch(HerdrCli::from_env(), herdr_client::config_dir(), &name, ctx)
    }

    /// Starts presenting `target` in the background, running `cli` to check that Herdr is present
    /// and compatible first. Unlike [`Self::new`] it never starts a server, so the session must
    /// already be running.
    #[cfg(test)]
    pub(crate) fn connect(
        cli: HerdrCli,
        target: herdr_client::SessionTarget,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        Self::connect_with_policy(cli, target, herdr_client::ReconnectPolicy::default(), ctx)
    }

    /// Like [`Self::connect`], retrying a lost connection as `policy` says.
    #[cfg(test)]
    pub(crate) fn connect_with_policy(
        cli: HerdrCli,
        target: herdr_client::SessionTarget,
        policy: herdr_client::ReconnectPolicy,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let mut sessions = SessionSelection::new(cli.clone(), herdr_client::config_dir());
        sessions.select(target.name().clone());
        let mut model = Self {
            presented: PresentedSession::connecting(),
            sessions,
            live_session: None,
            agents: AgentTracking::default(),
        };
        model.go_live(cli, target, policy, ctx);
        model
    }

    /// A model presenting a session that never connects.
    #[cfg(test)]
    pub(crate) fn never_connected() -> Self {
        Self {
            presented: PresentedSession::connecting(),
            sessions: SessionSelection::new(HerdrCli::new("herdr"), herdr_client::config_dir()),
            live_session: None,
            agents: AgentTracking::default(),
        }
    }

    fn failed(failure: ConnectionFailure, sessions: SessionSelection) -> Self {
        log::error!("Could not present the Herdr session: {failure}");
        Self {
            presented: PresentedSession::failed(failure),
            sessions,
            live_session: None,
            agents: AgentTracking::default(),
        }
    }

    pub(crate) fn presented(&self) -> &PresentedSession {
        &self.presented
    }

    fn set_presented(&mut self, presented: PresentedSession, ctx: &mut ModelContext<Self>) {
        if presented.connection != self.presented.connection {
            match &presented.connection {
                ConnectionStatus::Failed(failure) => {
                    log::error!("Could not present the Herdr session: {failure}");
                }
                ConnectionStatus::Refused(refusal) => {
                    log::error!("Refusing to connect to Herdr: {refusal}");
                }
                ConnectionStatus::Reconnecting(failure) => {
                    log::warn!("The Herdr session is unreachable, retrying: {failure}");
                }
                ConnectionStatus::Disconnected(failure) => {
                    log::error!("Still cannot reach the Herdr session, retrying: {failure}");
                }
                ConnectionStatus::Connecting | ConnectionStatus::Connected => {}
            }
        }
        self.presented = presented;
        ctx.emit(HerdrSessionEvent::Presented);
    }
}

impl Entity for HerdrSessionModel {
    type Event = HerdrSessionEvent;
}

impl SingletonEntity for HerdrSessionModel {}
