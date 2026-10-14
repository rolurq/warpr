//! Keeping the Herdr session's agents current next to the session itself.

use herdr_client::{
    AgentJump, HerdrCli, LiveAgents, PaneId, PresentedAgents, ReconnectPolicy, SessionName,
    SessionTarget,
};
use warpui::ModelContext;

use super::{HerdrSessionEvent, HerdrSessionModel};

/// The agents of the presented session, and what keeps them current.
pub(super) struct AgentTracking {
    presented: PresentedAgents,
    live: Option<LiveAgents>,
}

impl Default for AgentTracking {
    fn default() -> Self {
        Self {
            presented: PresentedAgents::connecting(),
            live: None,
        }
    }
}

impl HerdrSessionModel {
    pub(crate) fn agents(&self) -> &PresentedAgents {
        &self.agents.presented
    }

    /// Where to take the user to reach the agent in `pane`, as the session presents it now.
    /// Navigating there is local view state: nothing is asked of Herdr.
    pub(crate) fn agent_jump_target(&self, pane: &PaneId) -> Option<AgentJump> {
        self.agents
            .presented
            .agent(pane)?
            .jump_target(&self.presented)
    }

    /// Takes the user to the pane of the agent in `pane`, if the session presents it. Windows
    /// follow the [`HerdrSessionEvent::JumpedToAgent`] this emits; nothing is sent to Herdr.
    pub(crate) fn jump_to_agent(&mut self, pane: &PaneId, ctx: &mut ModelContext<Self>) {
        if let Some(jump) = self.agent_jump_target(pane) {
            ctx.emit(HerdrSessionEvent::JumpedToAgent(jump));
        }
    }

    /// Follows the agents of `target`, replacing whatever was followed before.
    pub(super) fn track_agents(
        &mut self,
        cli: HerdrCli,
        target: SessionTarget,
        policy: ReconnectPolicy,
        ctx: &mut ModelContext<Self>,
    ) {
        let name = target.name().clone();
        let (updates_tx, updates_rx) = async_channel::unbounded();
        self.agents.live = Some(LiveAgents::start(cli, target, policy, move |presented| {
            // Only fails once the model is gone, when nobody is left to present to.
            let _ = updates_tx.try_send(presented);
        }));
        let _ = ctx.spawn_stream_local(
            updates_rx,
            move |me, presented, ctx| me.apply_agents_update(&name, presented, ctx),
            |_, _| {},
        );
    }

    /// Stops following the agents, which are unknown until [`Self::track_agents`] is called again.
    pub(super) fn stop_tracking_agents(&mut self, ctx: &mut ModelContext<Self>) {
        self.agents.live = None;
        self.set_agents(PresentedAgents::connecting(), ctx);
    }

    /// Applies an update from the agents of session `name` unless the user has switched to
    /// another session since, which makes it stale.
    fn apply_agents_update(
        &mut self,
        name: &SessionName,
        presented: PresentedAgents,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.current_session() == Some(name) {
            self.set_agents(presented, ctx);
        }
    }

    fn set_agents(&mut self, presented: PresentedAgents, ctx: &mut ModelContext<Self>) {
        if presented == self.agents.presented {
            return;
        }
        self.agents.presented = presented;
        ctx.emit(HerdrSessionEvent::AgentsPresented);
    }
}
