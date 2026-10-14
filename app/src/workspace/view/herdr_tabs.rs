use std::collections::HashMap;
use std::sync::Arc;

use herdr_client::ConnectionStatus;
use warp_core::features::FeatureFlag;
use warpui::{SingletonEntity as _, ViewContext};

use super::Workspace;
use crate::app_state::{
    HerdrWorkspaceContents, HerdrWorkspaceSnapshot, LeafContents, LeafSnapshot, PaneNodeSnapshot,
};
use crate::herdr::{HerdrSessionEvent, HerdrSessionModel};
use crate::pane_group::PanesLayout;

impl Workspace {
    fn presents_herdr_workspaces(ctx: &ViewContext<Self>) -> bool {
        FeatureFlag::HerdrWorkspaceTabs.is_enabled()
            && ctx.has_singleton_model::<HerdrSessionModel>()
    }

    pub(super) fn subscribe_to_herdr_session(ctx: &mut ViewContext<Self>) {
        if !Self::presents_herdr_workspaces(ctx) {
            return;
        }
        ctx.subscribe_to_model(
            &HerdrSessionModel::handle(ctx),
            |me, _, event, ctx| match event {
                HerdrSessionEvent::Presented => me.present_herdr_workspaces(ctx),
                HerdrSessionEvent::SessionsListed => {}
            },
        );
    }

    /// Replaces this window's tabs with one top-level tab per Herdr workspace and selects the
    /// workspace Herdr has focused, or, when warpr refuses to connect to Herdr, with a single tab
    /// that says why. Leaves the tabs alone while Herdr is connecting, has no workspaces or could
    /// not be reached, so the window is never left without a tab.
    pub(super) fn present_herdr_workspaces(&mut self, ctx: &mut ViewContext<Self>) {
        if !Self::presents_herdr_workspaces(ctx) {
            return;
        }
        let presented = HerdrSessionModel::as_ref(ctx).presented().clone();
        let (tab_contents, focused_index) = match &presented.connection {
            ConnectionStatus::Connected if !presented.workspaces.is_empty() => {
                let contents: Vec<_> = presented
                    .workspaces
                    .iter()
                    .map(|workspace| {
                        HerdrWorkspaceContents::Workspace(HerdrWorkspaceSnapshot {
                            workspace_id: workspace.id.clone(),
                            label: workspace.label.clone(),
                        })
                    })
                    .collect();
                (contents, presented.focused_workspace_index())
            }
            ConnectionStatus::Refused(refusal) => (
                vec![HerdrWorkspaceContents::Refused(refusal.clone())],
                Some(0),
            ),
            ConnectionStatus::Connecting
            | ConnectionStatus::Connected
            | ConnectionStatus::Failed(_) => return,
        };

        let replaced_pane_groups: Vec<_> =
            self.tabs.iter().map(|tab| tab.pane_group.id()).collect();

        let mut workspace_pane_groups = Vec::with_capacity(tab_contents.len());
        for contents in tab_contents {
            let title = contents.title().to_owned();
            let panes_layout =
                PanesLayout::Snapshot(Box::new(PaneNodeSnapshot::Leaf(LeafSnapshot {
                    is_focused: true,
                    custom_vertical_tabs_title: Some(title.clone()),
                    contents: LeafContents::HerdrWorkspace(contents),
                })));
            self.add_tab_with_pane_layout(panes_layout, Arc::new(HashMap::new()), Some(title), ctx);
            workspace_pane_groups.push(self.active_tab_pane_group().id());
        }

        for pane_group_id in replaced_pane_groups {
            if let Some(index) = self.tab_index_for_pane_group(pane_group_id) {
                self.remove_tab(index, false, true, ctx);
            }
        }

        let focused_pane_group = focused_index.and_then(|index| workspace_pane_groups.get(index));
        if let Some(index) = focused_pane_group.and_then(|id| self.tab_index_for_pane_group(*id)) {
            self.activate_tab(index, ctx);
        }
        ctx.notify();
    }

    fn tab_index_for_pane_group(&self, pane_group_id: warpui::EntityId) -> Option<usize> {
        self.tabs
            .iter()
            .position(|tab| tab.pane_group.id() == pane_group_id)
    }
}

#[cfg(all(test, unix))]
#[path = "herdr_tabs_tests.rs"]
mod tests;
