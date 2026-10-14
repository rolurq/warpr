use crate::{ConnectionFailure, HerdrCli, PresentedSession, ReconnectPolicy, SessionTarget};

/// Herdr sessions are only reachable over unix sockets, so this reports the platform as
/// unsupported and never connects.
pub struct LiveSession;

impl LiveSession {
    pub fn start(
        _cli: HerdrCli,
        _target: SessionTarget,
        _policy: ReconnectPolicy,
        mut on_update: impl FnMut(PresentedSession) + Send + 'static,
    ) -> Self {
        on_update(PresentedSession::failed(
            ConnectionFailure::UnsupportedPlatform,
        ));
        Self
    }

    pub fn reconnect(&self) {}
}
