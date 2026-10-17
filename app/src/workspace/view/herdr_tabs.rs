use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use herdr_client::{
    AgentJump, ConnectionStatus, HerdrTabId, NewWorktree, PresentedSession, WorkspaceId,
};
use warp_core::features::FeatureFlag;
use warp_util::worktree_names::generate_worktree_branch_name;
use warpui::elements::{
    Align, Border, ChildView, Clipped, Container, Element, Expanded, Flex, Hoverable,
    MouseStateHandle, ParentElement as _, Text,
};
use warpui::platform::Cursor;
use warpui::{
    AppContext, EntityId, SingletonEntity as _, TypedActionView as _, ViewContext, ViewHandle,
};

use super::Workspace;
use crate::app_state::{
    HerdrWorkspaceContents, HerdrWorkspaceSnapshot, LeafContents, LeafSnapshot, PaneNodeSnapshot,
};
use crate::appearance::Appearance;
use crate::herdr::repository_root::repository_root;
use crate::herdr::{
    HerdrBarTab, HerdrSessionEvent, HerdrSessionModel, HerdrWorkspaceView, HerdrWorkspaceViewAction,
};
use crate::pane_group::PanesLayout;
use crate::settings::AISettings;
use crate::util::git::list_local_branches_sync;
use crate::view_components::DismissibleToast;
use crate::workspace::WorkspaceAction;
use crate::workspace::tab_group::TabGroup;

const TAB_FONT_SIZE: f32 = 13.;
const TAB_HORIZONTAL_PADDING: f32 = 12.;
const TAB_VERTICAL_PADDING: f32 = 6.;

/// The repositories of the session's workspaces.
pub(super) struct HerdrRepositories {
    /// Each repository of the session's workspaces once, as the root of its main checkout.
    pub(super) roots: Vec<PathBuf>,
    pub(super) preselected: Option<PathBuf>,
}

impl HerdrRepositories {
    /// Herdr reports the repository of a workspace only once a worktree of it exists, so a
    /// workspace without that is looked up from its directory.
    fn of(presented: &PresentedSession, shown: Option<&WorkspaceId>) -> Self {
        let root_of = |workspace: &WorkspaceId| {
            presented
                .repository_of(workspace)
                .map(|repository| repository.root)
                .or_else(|| repository_root(presented.workspace(workspace)?.directory()?))
        };
        let preselected = shown
            .into_iter()
            .chain(presented.focused_workspace.as_ref())
            .find_map(root_of);
        let mut roots: Vec<PathBuf> = presented
            .repositories()
            .into_iter()
            .map(|repository| repository.root)
            .collect();
        for workspace in presented
            .workspaces
            .iter()
            .filter(|workspace| workspace.worktree.is_none())
        {
            if let Some(root) = root_of(&workspace.id)
                && !roots.contains(&root)
            {
                roots.push(root);
            }
        }
        Self { roots, preselected }
    }
}

/// What the tab bar keeps between renders to draw the Herdr tabs it lists.
#[derive(Default)]
pub(super) struct HerdrTabBarState {
    /// Filled while rendering, because Herdr decides which tabs exist and `render` only has
    /// shared access to the workspace.
    tab_mouse_states: RefCell<HashMap<HerdrTabId, MouseStateHandle>>,
    new_tab_mouse_state: MouseStateHandle,
}

impl HerdrTabBarState {
    /// The Herdr tabs in a row, followed by the button that creates one.
    fn render(&self, tabs: &[HerdrBarTab], appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let mut mouse_states = self.tab_mouse_states.borrow_mut();
        let tabs = tabs.iter().map(|tab| {
            let is_shown = tab.is_shown;
            let tab_id = tab.id.clone();
            let mouse_state = mouse_states.entry(tab.id.clone()).or_default().clone();
            Hoverable::new(mouse_state, |state| {
                let background = if is_shown {
                    theme.surface_2()
                } else if state.is_hovered() {
                    theme.surface_1()
                } else {
                    theme.background()
                };
                let text_color = if is_shown {
                    theme.main_text_color(background)
                } else {
                    theme.sub_text_color(background)
                };
                let label = Text::new_inline(
                    tab.label.clone(),
                    appearance.ui_font_family(),
                    TAB_FONT_SIZE,
                )
                .with_color(text_color.into())
                .finish();
                let underline = if is_shown {
                    Border::bottom(2.).with_border_fill(theme.accent())
                } else {
                    Border::bottom(2.).with_border_fill(background)
                };
                Container::new(label)
                    .with_horizontal_padding(TAB_HORIZONTAL_PADDING)
                    .with_vertical_padding(TAB_VERTICAL_PADDING)
                    .with_background(background)
                    .with_border(underline)
                    .finish()
            })
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(WorkspaceAction::SelectHerdrTab(tab_id.clone()));
            })
            .with_cursor(Cursor::PointingHand)
            .finish()
        });
        Flex::row()
            .with_children(tabs)
            .with_child(self.render_new_tab_button(appearance))
            .finish()
    }

    fn render_new_tab_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        Hoverable::new(self.new_tab_mouse_state.clone(), |state| {
            let background = if state.is_hovered() {
                theme.surface_1()
            } else {
                theme.background()
            };
            let plus = Text::new_inline("+".to_owned(), appearance.ui_font_family(), TAB_FONT_SIZE)
                .with_color(theme.sub_text_color(background).into())
                .finish();
            Container::new(plus)
                .with_horizontal_padding(TAB_HORIZONTAL_PADDING)
                .with_vertical_padding(TAB_VERTICAL_PADDING)
                .with_background(background)
                .with_border(Border::bottom(2.).with_border_fill(background))
                .finish()
        })
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(WorkspaceAction::CreateHerdrTab);
        })
        .with_cursor(Cursor::PointingHand)
        .finish()
    }
}

impl Workspace {
    /// Whether the user may create and rearrange tab groups. Herdr windows present Herdr's own
    /// groupings and own the order, so there the user may not.
    pub(super) fn user_tab_groups_enabled(app: &AppContext) -> bool {
        FeatureFlag::GroupedTabs.is_enabled() && !crate::herdr::presents_herdr_workspaces(app)
    }

    fn presents_herdr_workspaces(ctx: &AppContext) -> bool {
        crate::herdr::presents_herdr_workspaces(ctx)
    }

    /// The view of the Herdr workspace this window shows.
    fn active_herdr_workspace_view(
        &self,
        ctx: &AppContext,
    ) -> Option<ViewHandle<HerdrWorkspaceView>> {
        self.active_tab_pane_group()
            .as_ref(ctx)
            .herdr_workspace_view(ctx)
    }

    /// The Herdr tabs the tab bar lists: those of the workspace this window shows, none while the
    /// agent panel covers the workspaces or the window shows no Herdr workspace.
    pub(super) fn herdr_bar_tabs(&self, ctx: &AppContext) -> Vec<HerdrBarTab> {
        if !Self::presents_herdr_workspaces(ctx)
            || self.current_workspace_state.is_agent_management_view_open
        {
            return Vec::new();
        }
        self.active_herdr_workspace_view(ctx)
            .map(|view| view.as_ref(ctx).bar_tabs(ctx))
            .unwrap_or_default()
    }

    /// The tab bar's Herdr tabs as a flexible child of the header row, or `None` when there are
    /// none to list.
    pub(super) fn render_herdr_bar_tabs(
        &self,
        appearance: &Appearance,
        ctx: &AppContext,
    ) -> Option<Box<dyn Element>> {
        let tabs = self.herdr_bar_tabs(ctx);
        if tabs.is_empty() {
            return None;
        }
        let strip = self.herdr_tab_bar.render(&tabs, appearance);
        // Twice the weight of the header's search bar slot, so the tabs get most of the room the
        // toolbar buttons and the right-hand controls leave.
        Some(Expanded::new(2., Clipped::new(Align::new(strip).left().finish()).finish()).finish())
    }

    /// Hands `action` to the view of the Herdr workspace this window shows, which owns the Herdr
    /// tab selection and the request to create one.
    pub(super) fn forward_to_herdr_workspace_view(
        &mut self,
        action: HerdrWorkspaceViewAction,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(view) = self.active_herdr_workspace_view(ctx) else {
            return;
        };
        view.update(ctx, |view, ctx| view.handle_action(&action, ctx));
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

    /// Tells the Herdr session which workspace this window shows the user: none while the agent
    /// panel covers the tabs or the selected tab is not a workspace. Only warpr's own notion:
    /// Herdr is not told.
    pub(super) fn report_shown_herdr_workspace(&self, ctx: &mut ViewContext<Self>) {
        if !Self::presents_herdr_workspaces(ctx) {
            return;
        }
        let selected = self
            .tabs
            .get(self.active_tab_index)
            .map(|tab| tab.pane_group.id())
            .filter(|_| !self.current_workspace_state.is_agent_management_view_open);
        let workspace = selected.and_then(|selected| {
            self.herdr_workspace_tabs(ctx)
                .into_iter()
                .find_map(|(pane_group_id, workspace_id)| {
                    (pane_group_id == selected).then_some(workspace_id)
                })
        });
        let window_id = ctx.window_id();
        HerdrSessionModel::handle(ctx).update(ctx, |model, ctx| {
            model.select_workspace(window_id, workspace, ctx);
        });
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

    /// Asks Herdr to create a workspace in the session this window presents. Nothing is added
    /// here: the workspace gets its tab, and the focus, when the session presents it. A refused
    /// request is shown to the user.
    pub(super) fn create_herdr_workspace(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(target) = HerdrSessionModel::as_ref(ctx).session_target() else {
            self.show_herdr_error(
                "Could not create a workspace: no Herdr session is connected",
                ctx,
            );
            return;
        };
        ctx.spawn(
            async move { herdr_client::create_workspace(&target) },
            |me, result, ctx| {
                if let Err(failure) = result {
                    log::error!("Could not create a Herdr workspace: {failure}");
                    me.show_herdr_error(&format!("Could not create a workspace: {failure}"), ctx);
                }
            },
        );
    }

    /// The repositories to offer for a new worktree, with the repository of the workspace the
    /// window shows preselected, or of the one Herdr has focused when the window shows none.
    /// `None` in windows that do not present a Herdr session.
    pub(super) fn herdr_new_worktree_repositories(
        &self,
        ctx: &AppContext,
    ) -> Option<HerdrRepositories> {
        if !Self::presents_herdr_workspaces(ctx) {
            return None;
        }
        let shown_tab = self.tabs.get(self.active_tab_index)?.pane_group.id();
        let shown =
            self.herdr_workspace_tabs(ctx)
                .into_iter()
                .find_map(|(pane_group_id, workspace_id)| {
                    (pane_group_id == shown_tab).then_some(workspace_id)
                });
        Some(HerdrRepositories::of(
            HerdrSessionModel::as_ref(ctx).presented(),
            shown.as_ref(),
        ))
    }

    /// Asks Herdr to create a linked worktree of the repository at `repo`, on a new branch from
    /// `base`, or from Herdr's default when `None`. The branch is named `branch_name`, or
    /// generated when `None`. Nothing is added
    /// here: the worktree workspace appears nested under its repository's, and focused, when the
    /// session presents it. A refused request is shown to the user.
    pub(super) fn create_herdr_worktree(
        &mut self,
        repo: &str,
        base: Option<&str>,
        branch_name: Option<&str>,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(target) = HerdrSessionModel::as_ref(ctx).session_target() else {
            self.show_herdr_error(
                "Could not create a worktree: no Herdr session is connected",
                ctx,
            );
            return;
        };
        let branch = match branch_name {
            Some(name) => name.to_owned(),
            None => {
                let branches = list_local_branches_sync(Path::new(repo));
                let branches: HashSet<_> = branches.iter().map(String::as_str).collect();
                generate_worktree_branch_name(&branches)
            }
        };
        let worktree = NewWorktree {
            repo: PathBuf::from(repo),
            branch,
            base: base.map(str::to_owned),
            path: None,
            label: None,
        };
        ctx.spawn(
            async move { herdr_client::create_worktree(&target, &worktree) },
            |me, result, ctx| {
                if let Err(failure) = result {
                    log::error!("Could not create a Herdr worktree: {failure}");
                    me.show_herdr_error(&format!("Could not create a worktree: {failure}"), ctx);
                }
            },
        );
    }

    fn show_herdr_error(&mut self, message: &str, ctx: &mut ViewContext<Self>) {
        self.toast_stack.update(ctx, |toast_stack, ctx| {
            toast_stack.add_ephemeral_toast(DismissibleToast::error(message.to_owned()), ctx);
        });
    }

    /// Brings this window's top-level tabs in line with what Herdr presents: one tab per
    /// workspace, in Herdr's order, or a single tab that says why when warpr refuses to connect
    /// to Herdr. A tab whose workspace is still there is kept, so what the user is looking at
    /// survives other workspaces coming and going.
    ///
    /// The window's own tabs are replaced the first time workspaces are presented, and the one
    /// Herdr has focused is selected. Later, the selected tab stays selected unless its workspace
    /// closed or a new workspace appears that Herdr has focused. Leaves the tabs alone while
    /// Herdr is connecting, has no workspaces or is unreachable, so the window is never left
    /// without a tab.
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
        self.report_shown_herdr_workspace(ctx);
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
        let mut focused_new_tab = None;
        for workspace in &presented.workspaces {
            if !tab_of_workspace.contains_key(&workspace.id) {
                let contents = HerdrWorkspaceContents::Workspace(HerdrWorkspaceSnapshot {
                    workspace_id: workspace.id.clone(),
                    label: workspace.label.clone(),
                });
                let pane_group_id = self.add_herdr_tab(contents, ctx);
                if !is_first_presentation
                    && presented.focused_workspace.as_ref() == Some(&workspace.id)
                {
                    focused_new_tab = Some(pane_group_id);
                }
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
            .workspace_groups()
            .into_iter()
            .flatten()
            .filter_map(|workspace| tab_of_workspace.get(workspace).copied())
            .collect();
        self.order_herdr_workspace_tabs(&ordered_tabs, ctx);
        self.group_herdr_workspace_tabs(presented, &tab_of_workspace, ctx);

        let selected_tab = focused_new_tab
            .or_else(|| {
                selected_before
                    .filter(|_| !is_first_presentation)
                    .filter(|pane_group_id| self.tab_index_for_pane_group(*pane_group_id).is_some())
            })
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
    fn herdr_tab_contents(&self, ctx: &AppContext) -> Vec<(EntityId, HerdrWorkspaceContents)> {
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
    fn herdr_workspace_tabs(&self, ctx: &AppContext) -> Vec<(EntityId, WorkspaceId)> {
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

    /// Presents each Herdr workspace as a tab group named after it, holding the workspace's own
    /// tab and, after it, the linked worktrees nested under it. A lead keeps its group between
    /// presentations so the group's color and collapsed state survive; groups left without
    /// members are dropped.
    fn group_herdr_workspace_tabs(
        &mut self,
        presented: &PresentedSession,
        tab_of_workspace: &HashMap<WorkspaceId, EntityId>,
        ctx: &mut ViewContext<Self>,
    ) {
        if !FeatureFlag::GroupedTabs.is_enabled() {
            return;
        }
        let mut claimed = HashSet::new();
        for members in presented.workspace_groups() {
            let Some(lead) = members.first().copied() else {
                continue;
            };
            let reusable = tab_of_workspace
                .get(lead)
                .and_then(|pane_group_id| self.tab_index_for_pane_group(*pane_group_id))
                .and_then(|index| self.tabs[index].group_id)
                .filter(|group_id| {
                    self.tab_groups.contains_key(group_id) && !claimed.contains(group_id)
                });
            let group_id = reusable.unwrap_or_else(|| {
                let group = TabGroup::new();
                let group_id = group.id;
                self.tab_groups.insert(group_id, group);
                group_id
            });
            claimed.insert(group_id);
            if let (Some(group), Some(workspace)) = (
                self.tab_groups.get_mut(&group_id),
                presented.workspace(lead),
            ) {
                group.name = Some(workspace.label.clone());
            }
            for member in members {
                let index = tab_of_workspace
                    .get(member)
                    .and_then(|pane_group_id| self.tab_index_for_pane_group(*pane_group_id));
                if let Some(tab) = index.and_then(|index| self.tabs.get_mut(index)) {
                    tab.group_id = Some(group_id);
                    tab.pinned = false;
                }
            }
        }
        let in_use: HashSet<_> = self.tabs.iter().filter_map(|tab| tab.group_id).collect();
        self.tab_groups
            .retain(|group_id, _| in_use.contains(group_id));
        ctx.notify();
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
