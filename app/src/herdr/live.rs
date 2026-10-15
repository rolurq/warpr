//! Keeping the presented session current once a connection has been made.

use herdr_client::{
    HerdrCli, LiveSession, PresentedSession, ReconnectPolicy, SessionName, SessionTarget,
};
use warpui::ModelContext;

use super::HerdrSessionModel;

impl HerdrSessionModel {
    /// Keeps presenting `target` as Herdr changes it, replacing whatever was kept current
    /// before. `cli` gates every connection attempt, including the reconnects of a lost one.
    pub(super) fn go_live(
        &mut self,
        cli: HerdrCli,
        target: SessionTarget,
        policy: ReconnectPolicy,
        ctx: &mut ModelContext<Self>,
    ) {
        self.track_agents(cli.clone(), target.clone(), policy, ctx);
        let name = target.name().clone();
        self.sessions.live_target = Some(target.clone());
        let (updates_tx, updates_rx) = async_channel::unbounded();
        self.live_session = Some(LiveSession::start(cli, target, policy, move |presented| {
            // Only fails once the model is gone, when nobody is left to present to.
            let _ = updates_tx.try_send(presented);
        }));
        let _ = ctx.spawn_stream_local(
            updates_rx,
            move |me, presented, ctx| me.apply_live_update(&name, presented, ctx),
            |_, _| {},
        );
    }

    /// Applies an update from the live session of `name` unless the user has switched to another
    /// session since, which makes it stale.
    fn apply_live_update(
        &mut self,
        name: &SessionName,
        presented: PresentedSession,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.current_session() == Some(name) {
            self.set_presented(presented, ctx);
        }
    }
}
