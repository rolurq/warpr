//! Which Herdr session warpr presents, and which others exist to switch to.

use std::path::PathBuf;

use herdr_client::{
    ConnectionFailure, ConnectionStatus, HerdrCli, ListedSession, PresentedSession,
    ReconnectPolicy, SessionName, SessionTarget, launch_session, list_sessions, open_session,
};
use warpui::ModelContext;

use super::agents::AgentTracking;
use super::{HerdrSessionEvent, HerdrSessionModel};

/// The picker's side of [`HerdrSessionModel`]: the session being presented and Herdr's listing.
pub(super) struct SessionSelection {
    cli: HerdrCli,
    /// Where Herdr keeps session sockets. `None` when the environment gives no home directory,
    /// in which case no session can be located.
    config_dir: Option<PathBuf>,
    current: Option<SessionName>,
    listed: Vec<ListedSession>,
}

impl SessionSelection {
    pub(super) fn new(cli: HerdrCli, config_dir: Option<PathBuf>) -> Self {
        Self {
            cli,
            config_dir,
            current: None,
            listed: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(super) fn select(&mut self, name: SessionName) {
        self.current = Some(name);
    }

    fn target(&self, name: &SessionName) -> Result<SessionTarget, ConnectionFailure> {
        let config_dir =
            self.config_dir
                .as_deref()
                .ok_or_else(|| ConnectionFailure::SessionUnresolvable {
                    name: name.as_str().to_owned(),
                    reason: "neither XDG_CONFIG_HOME nor HOME is set".to_owned(),
                })?;
        Ok(SessionTarget::new(config_dir, name.clone()))
    }
}

impl HerdrSessionModel {
    /// Starts presenting the session called `session`: the server is started first if Herdr lists
    /// the session as stopped, and the default session is always brought up. Also reads Herdr's
    /// session listing for the picker.
    pub(super) fn launch(
        cli: HerdrCli,
        config_dir: Option<PathBuf>,
        session: &str,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let mut sessions = SessionSelection::new(cli, config_dir);
        let target = SessionName::new(session)
            .map_err(|error| ConnectionFailure::SessionUnresolvable {
                name: session.to_owned(),
                reason: error.to_string(),
            })
            .and_then(|name| {
                sessions.current = Some(name.clone());
                sessions.target(&name)
            });
        let target = match target {
            Ok(target) => target,
            Err(failure) => return Self::failed(failure, sessions),
        };

        let cli = sessions.cli.clone();
        ctx.spawn(
            async move {
                let presented = launch_session(&cli, &target);
                (target, presented)
            },
            |me, (target, presented), ctx| me.set_presented_for(target, presented, ctx),
        );
        let mut model = Self {
            presented: PresentedSession::connecting(),
            sessions,
            live_session: None,
            agents: AgentTracking::default(),
        };
        model.refresh_sessions(ctx);
        model
    }

    /// Herdr's sessions, in the order it lists them, as of the last [`Self::refresh_sessions`].
    pub(crate) fn listed_sessions(&self) -> &[ListedSession] {
        &self.sessions.listed
    }

    /// The session that is being presented or connected to.
    pub(crate) fn current_session(&self) -> Option<&SessionName> {
        self.sessions.current.as_ref()
    }

    /// What pane streams of the current session run with, or `None` while no session is chosen
    /// or it cannot be located.
    pub(super) fn stream_source(&self) -> Option<(HerdrCli, SessionTarget)> {
        let target = self.sessions.target(self.sessions.current.as_ref()?).ok()?;
        Some((self.sessions.cli.clone(), target))
    }

    /// Reads Herdr's session listing in the background. A listing that fails leaves the previous
    /// one in place and is logged.
    pub(crate) fn refresh_sessions(&mut self, ctx: &mut ModelContext<Self>) {
        let cli = self.sessions.cli.clone();
        ctx.spawn(async move { list_sessions(&cli) }, |me, listing, ctx| {
            match listing {
                Ok(listed) => me.sessions.listed = listed,
                Err(failure) => log::error!("Could not list the Herdr sessions: {failure}"),
            }
            ctx.emit(HerdrSessionEvent::SessionsListed);
        });
    }

    /// Presents the session called `name` instead of the current one, starting its server first
    /// if the last listing says it is stopped. Only sessions Herdr lists can be switched to.
    pub(crate) fn switch_session(&mut self, name: &SessionName, ctx: &mut ModelContext<Self>) {
        let Some(state) = self
            .sessions
            .listed
            .iter()
            .find(|session| &session.name == name)
            .map(|session| session.state)
        else {
            return;
        };
        let already_presented = self.sessions.current.as_ref() == Some(name)
            && !matches!(self.presented.connection, ConnectionStatus::Failed(_));
        if already_presented {
            return;
        }

        self.sessions.current = Some(name.clone());
        self.live_session = None;
        self.stop_tracking_agents(ctx);
        let target = match self.sessions.target(name) {
            Ok(target) => target,
            Err(failure) => {
                self.set_presented(PresentedSession::failed(failure), ctx);
                return;
            }
        };
        self.set_presented(PresentedSession::connecting(), ctx);

        let cli = self.sessions.cli.clone();
        ctx.spawn(
            async move {
                let presented = open_session(&cli, &target, state);
                (target, presented)
            },
            |me, (target, presented), ctx| {
                me.set_presented_for(target, presented, ctx);
                // A session that was started is running now.
                me.refresh_sessions(ctx);
            },
        );
    }

    /// Applies `presented` unless the user has switched to another session while it was being
    /// prepared, which makes it stale. A connected session is kept current from then on.
    fn set_presented_for(
        &mut self,
        target: SessionTarget,
        presented: PresentedSession,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.sessions.current.as_ref() != Some(target.name()) {
            return;
        }
        let is_connected = presented.connection == ConnectionStatus::Connected;
        self.set_presented(presented, ctx);
        if is_connected {
            let cli = self.sessions.cli.clone();
            self.go_live(cli, target, ReconnectPolicy::default(), ctx);
        }
    }
}

#[cfg(all(test, unix))]
#[path = "sessions_tests.rs"]
mod tests;
