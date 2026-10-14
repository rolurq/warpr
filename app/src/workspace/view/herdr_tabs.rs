use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use herdr_client::{AgentJump, ConnectionStatus, PresentedSession, WorkspaceId};
use warp_core::features::FeatureFlag;
use warpui::elements::{ChildView, Element};
use warpui::{AppContext, EntityId, SingletonEntity as _, ViewContext};

use super::Workspace;
use crate::app_state::{
    HerdrWorkspaceContents, HerdrWorkspaceSnapshot, LeafContents, LeafSnapshot, PaneNodeSnapshot,
};
use crate::herdr::{HerdrSessionEvent, HerdrSessionModel};
use crate::pane_group::PanesLayout;
use crate::settings::AISettings;

impl Workspace {
    fn presents_herdr_workspaces(ctx: &ViewContext<Self>) -> bool {
        crate::herdr::presents_herdr_workspaces(ctx)
    }

    /// Whether the agent panel can be opened: Herdr's agents when this window presents a Herdr
    /// session, Warp's own agent management otherwise.
    pub(super) fn is_agent_panel_available(&self, app: &AppContext) -> bool {
        FeatureFlag::AgentManagementView.is_enabled()
            && (self.herdr_agent_panel.is_some() || AISettings::as_ref(app).is_any_ai_enabled(app))
    }

    pub(super) fn agent_management_content(&self) -> Box<dyn Element> {
        match &self.herdr_agent_panel {
            Some(panel) => ChildView::new(panel).finish(),
            None => ChildView::new(&self.agent_management_view).finish(),
        }
    }

    pub(super) fn focus_agent_management_view(&self, ctx: &mut ViewContext<Self>) {
        match &self.herdr_agent_panel {
            Some(panel) => ctx.focus(panel),
            None => ctx.focus(&self.agent_management_view),
        }
    }

    /// Shows the workspace of `jump`, closing the agent panel. Which Herdr tab and pane to show
    /// is up to that workspace's own view. Herdr's focus is left alone.
    fn jump_to_herdr_agent(&mut self, jump: &AgentJump, ctx: &mut ViewContext<Self>) {
        let tab_index = self
            .herdr_workspace_tabs(ctx)
            .into_iter()
            .find(|(_, workspace_id)| *workspace_id == jump.workspace)
            .and_then(|(pane_group_id, _)| self.tab_index_for_pane_group(pane_group_id));
        let Some(index) = tab_index else {
            return;
        };
        self.set_is_agent_management_view_open(false, ctx);
        self.activate_tab(index, ctx);
        ctx.notify();
    }

    pub(super) fn subscribe_to_herdr_session(ctx: &mut ViewContext<Self>) {
        if !Self::presents_herdr_workspaces(ctx) {
            return;
        }
        ctx.subscribe_to_model(
            &HerdrSessionModel::handle(ctx),
            |me, _, event, ctx| match event {
                HerdrSessionEvent::Presented => me.present_herdr_workspaces(ctx),
                HerdrSessionEvent::JumpedToAgent(jump) => me.jump_to_herdr_agent(jump, ctx),
                HerdrSessionEvent::SessionsListed | HerdrSessionEvent::AgentsPresented => {}
            },
        );
    }

    /// Brings this window's top-level tabs in line with what Herdr presents: one tab per
    /// workspace, in Herdr's order, or a single tab that says why when warpr refuses to connect
    /// to Herdr. A tab whose workspace is still there is kept, so what the user is looking at
    /// survives other workspaces coming and going.
    ///
    /// The window's own tabs are replaced the first time workspaces are presented, and the one
    /// Herdr has focused is selected. Later, the selected tab stays selected unless its workspace
    /// closed. Leaves the tabs alone while Herdr is connecting, has no workspaces or is
    /// unreachable, so the window is never left without a tab.
    pub(super) fn present_herdr_workspaces(&mut self, ctx: &mut ViewContext<Self>) {
        if !Self::presents_herdr_workspaces(ctx) {
            return;
        }
        let presented = HerdrSessionModel::as_ref(ctx).presented().clone();
        match &presented.connection {
            ConnectionStatus::Connected if !presented.workspaces.is_empty() => {
                self.reconcile_herdr_workspace_tabs(&presented, ctx);
            }
            ConnectionStatus::Refused(refusal) => {
                self.show_herdr_refusal(HerdrWorkspaceContents::Refused(refusal.clone()), ctx);
            }
            ConnectionStatus::Connecting
            | ConnectionStatus::Connected
            | ConnectionStatus::Reconnecting(_)
            | ConnectionStatus::Disconnected(_)
            | ConnectionStatus::Failed(_) => {}
        }
    }

    fn reconcile_herdr_workspace_tabs(
        &mut self,
        presented: &PresentedSession,
        ctx: &mut ViewContext<Self>,
    ) {
        let existing = self.herdr_workspace_tabs(ctx);
        let is_first_presentation = existing.is_empty();
        let selected_before = self
            .tabs
            .get(self.active_tab_index)
            .map(|tab| tab.pane_group.id());

        let still_open: HashSet<_> = presented.workspaces.iter().map(|w| &w.id).collect();
        let stale_tabs: Vec<_> = if is_first_presentation {
            self.tabs.iter().map(|tab| tab.pane_group.id()).collect()
        } else {
            existing
                .iter()
                .filter(|(_, workspace_id)| !still_open.contains(workspace_id))
                .map(|(pane_group_id, _)| *pane_group_id)
                .collect()
        };

        let mut tab_of_workspace: HashMap<_, _> = existing
            .into_iter()
            .map(|(pane_group_id, workspace_id)| (workspace_id, pane_group_id))
            .collect();
        for workspace in &presented.workspaces {
            if !tab_of_workspace.contains_key(&workspace.id) {
                let contents = HerdrWorkspaceContents::Workspace(HerdrWorkspaceSnapshot {
                    workspace_id: workspace.id.clone(),
                    label: workspace.label.clone(),
                });
                let pane_group_id = self.add_herdr_tab(contents, ctx);
                tab_of_workspace.insert(workspace.id.clone(), pane_group_id);
            }
        }

        // Closing tabs only after the new ones exist keeps the window from ever being tab-less,
        // which would close it.
        for pane_group_id in stale_tabs {
            if let Some(index) = self.tab_index_for_pane_group(pane_group_id) {
                self.remove_tab(index, false, true, ctx);
            }
        }

        let ordered_tabs: Vec<_> = presented
            .workspaces
            .iter()
            .filter_map(|workspace| tab_of_workspace.get(&workspace.id).copied())
            .collect();
        self.order_herdr_workspace_tabs(&ordered_tabs, ctx);

        let selected_tab = selected_before
            .filter(|_| !is_first_presentation)
            .filter(|pane_group_id| self.tab_index_for_pane_group(*pane_group_id).is_some())
            .or_else(|| {
                let focused = presented.focused_workspace.as_ref()?;
                tab_of_workspace.get(focused).copied()
            })
            .or_else(|| ordered_tabs.first().copied());
        if let Some(index) = selected_tab.and_then(|id| self.tab_index_for_pane_group(id))
            && index != self.active_tab_index
        {
            self.activate_tab(index, ctx);
        }
        ctx.notify();
    }

    /// Replaces every tab of this window with one that shows `refusal`, unless that is all the
    /// window already shows.
    fn show_herdr_refusal(&mut self, refusal: HerdrWorkspaceContents, ctx: &mut ViewContext<Self>) {
        let is_shown = self.tabs.len() == 1
            && self
                .herdr_tab_contents(ctx)
                .first()
                .is_some_and(|(_, contents)| *contents == refusal);
        if is_shown {
            return;
        }
        let replaced: Vec<_> = self.tabs.iter().map(|tab| tab.pane_group.id()).collect();
        self.add_herdr_tab(refusal, ctx);
        for pane_group_id in replaced {
            if let Some(index) = self.tab_index_for_pane_group(pane_group_id) {
                self.remove_tab(index, false, true, ctx);
            }
        }
        ctx.notify();
    }

    /// The tabs that show Herdr contents, in tab order, with what each shows.
    fn herdr_tab_contents(
        &self,
        ctx: &ViewContext<Self>,
    ) -> Vec<(EntityId, HerdrWorkspaceContents)> {
        self.tabs
            .iter()
            .filter_map(|tab| {
                let pane_group = tab.pane_group.as_ref(ctx);
                match pane_group.snapshot(ctx) {
                    PaneNodeSnapshot::Leaf(LeafSnapshot {
                        contents: LeafContents::HerdrWorkspace(contents),
                        ..
                    }) => Some((tab.pane_group.id(), contents)),
                    PaneNodeSnapshot::Leaf(_) | PaneNodeSnapshot::Branch(_) => None,
                }
            })
            .collect()
    }

    /// The tabs that stand for Herdr workspaces, in tab order, with the workspace each stands for.
    fn herdr_workspace_tabs(&self, ctx: &ViewContext<Self>) -> Vec<(EntityId, WorkspaceId)> {
        self.herdr_tab_contents(ctx)
            .into_iter()
            .filter_map(|(pane_group_id, contents)| match contents {
                HerdrWorkspaceContents::Workspace(workspace) => {
                    Some((pane_group_id, workspace.workspace_id))
                }
                HerdrWorkspaceContents::Refused(_) => None,
            })
            .collect()
    }

    /// Adds a tab showing `contents`, which also selects it, and returns its pane group.
    fn add_herdr_tab(
        &mut self,
        contents: HerdrWorkspaceContents,
        ctx: &mut ViewContext<Self>,
    ) -> EntityId {
        let panes_layout = PanesLayout::Snapshot(Box::new(PaneNodeSnapshot::Leaf(LeafSnapshot {
            is_focused: true,
            custom_vertical_tabs_title: Some(contents.title().to_owned()),
            contents: LeafContents::HerdrWorkspace(contents),
        })));
        self.add_tab_with_pane_layout(panes_layout, Arc::new(HashMap::new()), None, ctx);
        self.active_tab_pane_group().id()
    }

    /// Moves the workspace tabs `ordered`, which are all open, into that order as one run that
    /// starts where the first of them currently sits.
    fn order_herdr_workspace_tabs(&mut self, ordered: &[EntityId], ctx: &mut ViewContext<Self>) {
        let Some(first) = self
            .tabs
            .iter()
            .position(|tab| ordered.contains(&tab.pane_group.id()))
        else {
            return;
        };
        for (offset, pane_group_id) in ordered.iter().enumerate() {
            if let Some(index) = self.tab_index_for_pane_group(*pane_group_id) {
                self.hop_tab_to_index(index, first + offset, ctx);
            }
        }
    }

    fn tab_index_for_pane_group(&self, pane_group_id: EntityId) -> Option<usize> {
        self.tabs
            .iter()
            .position(|tab| tab.pane_group.id() == pane_group_id)
    }
}

#[cfg(all(test, unix))]
#[path = "herdr_tabs_tests.rs"]
mod tests;
