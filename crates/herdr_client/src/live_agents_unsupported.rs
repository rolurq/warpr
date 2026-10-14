use crate::{ConnectionFailure, HerdrCli, PresentedAgents, ReconnectPolicy, SessionTarget};

/// Herdr sessions are only reachable over unix sockets, so this reports the platform as
/// unsupported and never connects.
pub struct LiveAgents;

impl LiveAgents {
    pub fn start(
        _cli: HerdrCli,
        _target: SessionTarget,
        _policy: ReconnectPolicy,
        mut on_update: impl FnMut(PresentedAgents) + Send + 'static,
    ) -> Self {
        on_update(PresentedAgents::failed(
            ConnectionFailure::UnsupportedPlatform,
        ));
        Self
    }

    pub fn reconnect(&self) {}
}
