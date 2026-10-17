use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use herdr_client::{
    AgentJump, ConnectionStatus, Contest, HerdrTabId, PaneId, PaneInput, PaneOwnership,
    PresentedHerdrTab, PresentedPane, PresentedWorkspace,
};
use repo_metadata::repositories::{DetectedRepositories, RepoDetectionSource};
use warpui::elements::{
    Align, Border, ConstrainedBox, Container, CrossAxisAlignment, Expanded, Flex,
    MainAxisAlignment, MainAxisSize, ParentElement as _, Stack, Text,
};
use warpui::{
    AppContext, Element, Entity, EntityId, ModelHandle, SingletonEntity as _, TypedActionView,
    View, ViewContext, WindowId,
};

use super::directory_git::{DirectoryGitContext, HerdrDirectoryGit, HerdrDirectoryGitEvent};
use super::pane_chrome::{
    PANE_BORDER_WIDTH, PaneHeaderMouseStates, can_close_panes, dims_pane, render_pane_header,
};
use super::pane_contents::{HerdrPaneContentsRegistry, HerdrPaneContentsRegistryEvent};
use super::pane_frames::PaneFrames;
use super::pane_grid::{PaneGridElement, PaneGridStyle, PaneInputTarget};
use super::{HerdrSessionEvent, HerdrSessionModel};
use crate::app_state::HerdrWorkspaceContents;
use crate::appearance::Appearance;
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::pane::view;
use crate::pane_group::{BackingView, PaneConfiguration, PaneEvent};
use crate::settings::{PaneSettings, PaneSettingsChangedEvent};
use crate::util::repo_detection::{RepoDetectionSessionType, detect_possible_git_repo};

const LABEL_FONT_SIZE: f32 = 20.;
const DETAIL_FONT_SIZE: f32 = 12.;
const ROW_HORIZONTAL_PADDING: f32 = 12.;
const ROW_VERTICAL_PADDING: f32 = 6.;
const REFUSAL_MESSAGE_FONT_SIZE: f32 = 14.;
const REFUSAL_MESSAGE_MAX_WIDTH: f32 = 560.;

#[derive(Clone, Debug, PartialEq)]
pub enum HerdrWorkspaceViewAction {
    /// Shows a Herdr tab in this view. It only changes what is displayed: Herdr's own focus is
    /// left alone, so other Herdr clients see no change.
    SelectTab(HerdrTabId),
    /// Gives a pane the focus of this view, so that typing goes to it. Like `SelectTab` it is
    /// view state: Herdr is not told, and nothing is taken from another client.
    SelectPane(PaneId),
    /// Sends the clipboard's text to a pane's program.
    Paste(PaneId),
    /// Asks Herdr to create a Herdr tab in this workspace.
    CreateTab,
    /// Asks Herdr to close a pane. Nothing is removed here: the pane leaves the view once the
    /// session stops presenting it. A refused request is shown in the view.
    ClosePane(PaneId),
}

/// What the Warp sidebar shows for a Herdr workspace beyond its label.
pub struct HerdrSidebarDetails<'a> {
    /// The working directory of the selected pane, as Herdr reports it.
    pub directory: Option<&'a Path>,
    /// The git repository holding `directory`; `None` outside one.
    pub git: Option<DirectoryGitContext>,
}

/// A Herdr tab as the window's tab bar lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HerdrBarTab {
    pub id: HerdrTabId,
    pub label: String,
    /// Whether this is the Herdr tab the workspace's view shows.
    pub is_shown: bool,
}

/// The contents of the pane that stands for a Herdr workspace in its top-level tab, or for the
/// session when warpr refuses to connect to it.
pub struct HerdrWorkspaceView {
    contents: HerdrWorkspaceContents,
    /// The Herdr tab the user picked here, if any; Herdr's active tab is shown until then.
    selected_tab: Option<HerdrTabId>,
    /// The pane an agent jump took the user to, shown selected until the user picks another Herdr
    /// tab. Like `selected_tab` it is view state that Herdr never hears of.
    selected_pane: Option<PaneId>,
    /// Why the last request this view made to Herdr was refused, until the next one succeeds.
    request_error: Option<String>,
    pane_configuration: ModelHandle<PaneConfiguration>,
    /// The git repository of the selected pane's directory.
    directory_git: ModelHandle<HerdrDirectoryGit>,
    focus_handle: Option<PaneFocusHandle>,
    window_id: WindowId,
    view_id: EntityId,
    /// The directory last reported to the pane group.
    reported_directory: Option<PathBuf>,
    /// The mouse state of each pane header, so that hover and clicks survive a redraw.
    pane_header_states: RefCell<HashMap<PaneId, PaneHeaderMouseStates>>,
}

impl HerdrWorkspaceView {
    pub fn new(contents: HerdrWorkspaceContents, ctx: &mut ViewContext<Self>) -> Self {
        let title = contents.title().to_owned();
        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new(&title));
        if ctx.has_singleton_model::<HerdrSessionModel>() {
            ctx.subscribe_to_model(
                &HerdrSessionModel::handle(ctx),
                |me, _, event, ctx| match event {
                    HerdrSessionEvent::Presented => me.follow_session(ctx),
                    HerdrSessionEvent::JumpedToAgent(jump) => me.follow_jump(jump, ctx),
                    HerdrSessionEvent::SessionsListed | HerdrSessionEvent::AgentsPresented => {}
                },
            );
            if !ctx.has_singleton_model::<HerdrPaneContentsRegistry>() {
                ctx.add_singleton_model(HerdrPaneContentsRegistry::new);
            }
            ctx.subscribe_to_model(
                &HerdrPaneContentsRegistry::handle(ctx),
                |_, _, HerdrPaneContentsRegistryEvent::PaneChanged, ctx| ctx.notify(),
            );
        }
        let directory_git = ctx.add_model(|_| HerdrDirectoryGit::new());
        ctx.subscribe_to_model(
            &directory_git,
            |_, _, HerdrDirectoryGitEvent::Changed, ctx| {
                ctx.emit(PaneEvent::SidebarDetailsChanged);
            },
        );
        ctx.subscribe_to_model(&PaneSettings::handle(ctx), |_, _, event, ctx| {
            if matches!(
                event,
                PaneSettingsChangedEvent::ShouldDimInactivePanes { .. }
            ) {
                ctx.notify();
            }
        });
        let mut view = Self {
            contents,
            selected_tab: None,
            selected_pane: None,
            request_error: None,
            pane_configuration,
            directory_git,
            focus_handle: None,
            window_id: ctx.window_id(),
            view_id: ctx.view_id(),
            reported_directory: None,
            pane_header_states: RefCell::default(),
        };
        view.update_shown_panes(ctx);
        view.sync_sidebar_details(ctx);
        view
    }

    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    pub fn snapshot(&self) -> HerdrWorkspaceContents {
        self.contents.clone()
    }

    /// Shows the Herdr tab and pane of `jump` when it is in this workspace.
    fn follow_jump(&mut self, jump: &AgentJump, ctx: &mut ViewContext<Self>) {
        let HerdrWorkspaceContents::Workspace(workspace) = &self.contents else {
            return;
        };
        if workspace.workspace_id != jump.workspace {
            return;
        }
        self.selected_tab = Some(jump.herdr_tab.clone());
        self.selected_pane = Some(jump.pane.clone());
        self.report_viewed_panes(ctx);
        self.update_shown_panes(ctx);
        self.sync_sidebar_details(ctx);
        ctx.emit(PaneEvent::HerdrTabChanged);
        ctx.notify();
    }

    /// The panes this view shows now.
    fn shown_panes(&self, app: &AppContext) -> Vec<PaneId> {
        let shown = self
            .presented_workspace(app)
            .and_then(|workspace| workspace.selected_tab(self.selected_tab.as_ref()))
            .and_then(|tab| tab.layout.as_ref())
            .into_iter()
            .flat_map(|layout| layout.frames())
            .map(|pane| pane.id);
        shown.chain(self.selected_pane.clone()).collect()
    }

    /// Tells the session which panes of this workspace this window shows, so that finished work
    /// in them is not announced as unseen while the window has the user's attention. Only warpr's
    /// own notion: Herdr is not told.
    fn report_viewed_panes(&self, ctx: &mut ViewContext<Self>) {
        let HerdrWorkspaceContents::Workspace(workspace) = &self.contents else {
            return;
        };
        if !ctx.has_singleton_model::<HerdrSessionModel>() {
            return;
        }
        let workspace_id = workspace.workspace_id.clone();
        let panes = self.shown_panes(ctx);
        let window_id = ctx.window_id();
        HerdrSessionModel::handle(ctx).update(ctx, |model, ctx| {
            model.show_panes(window_id, workspace_id, panes, ctx);
        });
    }

    /// Takes the workspace's label from the session's presented state, where Herdr's renames
    /// land, and redraws.
    fn follow_session(&mut self, ctx: &mut ViewContext<Self>) {
        let label = self
            .presented_workspace(ctx)
            .map(|workspace| workspace.label.clone());
        if let Some(label) = label
            && let HerdrWorkspaceContents::Workspace(workspace) = &mut self.contents
            && label != workspace.label
        {
            workspace.label = label.clone();
            self.pane_configuration.update(ctx, |configuration, ctx| {
                configuration.set_title(label.clone(), ctx);
                configuration.set_custom_vertical_tabs_title(label, ctx);
            });
        }
        self.report_viewed_panes(ctx);
        self.sync_sidebar_details(ctx);
        self.update_shown_panes(ctx);
        ctx.notify();
    }

    /// Tells the pane registry which panes this view shows, those of the Herdr tab on screen,
    /// and reports a changed directory.
    fn update_shown_panes(&mut self, ctx: &mut ViewContext<Self>) {
        let shown = self.shown_panes(ctx);
        self.show_panes(shown, ctx);
        self.report_directory_change(ctx);
    }

    /// Emits [`PaneEvent::RepoChanged`] when [`Self::focused_pane_directory`] differs from the
    /// last one reported, and again once the git repository containing it has been detected.
    fn report_directory_change(&mut self, ctx: &mut ViewContext<Self>) {
        let directory = self.focused_pane_directory(ctx).map(Path::to_path_buf);
        if directory == self.reported_directory {
            return;
        }
        self.reported_directory = directory.clone();
        ctx.emit(PaneEvent::RepoChanged);
        if let Some(directory) = directory
            && ctx.has_singleton_model::<DetectedRepositories>()
        {
            let detection = detect_possible_git_repo(
                RepoDetectionSessionType::Local,
                &directory.to_string_lossy(),
                RepoDetectionSource::TerminalNavigation,
                ctx,
            );
            ctx.spawn(detection, |_, repository, ctx| {
                if repository.is_some() {
                    ctx.emit(PaneEvent::RepoChanged);
                }
            });
        }
    }

    fn show_panes(&mut self, panes: Vec<PaneId>, ctx: &mut ViewContext<Self>) {
        if !ctx.has_singleton_model::<HerdrPaneContentsRegistry>() {
            return;
        }
        let view_id = ctx.view_id();
        HerdrPaneContentsRegistry::handle(ctx).update(ctx, |registry, ctx| {
            registry.set_shown(view_id, panes, ctx);
        });
    }

    /// The working directory of the pane the user has selected in the Herdr tab this view shows,
    /// as Herdr reports it: that pane's `cwd`, or `None` when Herdr reports none, no Herdr tab
    /// is shown, or the selected pane is not on screen. Before the user selects a pane it is the
    /// pane Herdr has focused in that Herdr tab.
    ///
    /// This is the directory context of a Herdr workspace, which has no Warp session to ask.
    /// It is derived on each call from the presented session and this view's selection, so it
    /// is always current; the view is notified whenever either changes. Reading it never talks
    /// to Herdr, and `foreground_cwd` is deliberately not consulted.
    pub fn focused_pane_directory<'a>(&self, app: &'a AppContext) -> Option<&'a Path> {
        let layout = self
            .presented_workspace(app)?
            .selected_tab(self.selected_tab.as_ref())?
            .layout
            .as_ref()?;
        layout
            .panes
            .iter()
            .filter(|pane| !layout.zoomed || pane.id == layout.focused_pane)
            .find(|pane| self.is_selected(pane))?
            .cwd
            .as_deref()
    }

    fn is_selected(&self, pane: &PresentedPane) -> bool {
        match &self.selected_pane {
            Some(selected) => *selected == pane.id,
            None => pane.focused,
        }
    }

    /// What the Warp sidebar shows for this workspace beyond its label: the selected pane's
    /// directory and the git repository that directory is in.
    pub fn sidebar_details<'a>(&self, app: &'a AppContext) -> HerdrSidebarDetails<'a> {
        HerdrSidebarDetails {
            directory: self.focused_pane_directory(app),
            git: self.directory_git.as_ref(app).context(app),
        }
    }

    /// Points the git context at the selected pane's directory and tells the pane group when
    /// what the sidebar shows changed.
    fn sync_sidebar_details(&mut self, ctx: &mut ViewContext<Self>) {
        let directory = self.focused_pane_directory(ctx).map(Path::to_path_buf);
        let changed = self
            .directory_git
            .update(ctx, |git, ctx| git.set_directory(directory, ctx));
        if changed {
            ctx.emit(PaneEvent::SidebarDetailsChanged);
        }
    }

    /// This workspace as the Herdr session currently presents it.
    fn presented_workspace<'a>(&self, app: &'a AppContext) -> Option<&'a PresentedWorkspace> {
        let HerdrWorkspaceContents::Workspace(workspace) = &self.contents else {
            return None;
        };
        if !app.has_singleton_model::<HerdrSessionModel>() {
            return None;
        }
        HerdrSessionModel::as_ref(app)
            .presented()
            .workspace(&workspace.workspace_id)
    }

    /// The Herdr tabs of this workspace in Herdr's order, marking the one this view shows.
    pub fn bar_tabs(&self, ctx: &AppContext) -> Vec<HerdrBarTab> {
        let Some(workspace) = self.presented_workspace(ctx) else {
            return Vec::new();
        };
        let shown = workspace
            .selected_tab(self.selected_tab.as_ref())
            .map(|tab| &tab.id);
        workspace
            .herdr_tabs
            .iter()
            .map(|tab| HerdrBarTab {
                id: tab.id.clone(),
                label: tab.label.clone(),
                is_shown: shown == Some(&tab.id),
            })
            .collect()
    }

    fn render_workspace_label(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let font_family = appearance.ui_font_family();
        let title = Text::new_inline(
            self.contents.title().to_owned(),
            font_family,
            LABEL_FONT_SIZE,
        )
        .with_color(theme.main_text_color(theme.background()).into())
        .finish();
        let detail = match &self.contents {
            HerdrWorkspaceContents::Workspace(workspace) => Text::new_inline(
                format!("Herdr workspace {}", workspace.workspace_id.as_str()),
                font_family,
                DETAIL_FONT_SIZE,
            )
            .with_color(theme.sub_text_color(theme.background()).into())
            .finish(),
            HerdrWorkspaceContents::Refused(refusal) => ConstrainedBox::new(
                Text::new(refusal.to_string(), font_family, REFUSAL_MESSAGE_FONT_SIZE)
                    .with_color(theme.sub_text_color(theme.background()).into())
                    .finish(),
            )
            .with_max_width(REFUSAL_MESSAGE_MAX_WIDTH)
            .finish(),
        };

        Align::new(
            Flex::column()
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(title)
                .with_child(detail)
                .finish(),
        )
        .finish()
    }

    fn render_pane_frames(
        &self,
        tab: &PresentedHerdrTab,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let Some(layout) = &tab.layout else {
            return Align::new(self.render_workspace_label(appearance)).finish();
        };
        let panes = layout.frames();
        self.pane_header_states
            .borrow_mut()
            .retain(|id, _| panes.iter().any(|pane| pane.id == *id));
        let can_close = can_close_panes(layout);
        let dim_inactive_panes = *PaneSettings::as_ref(app).should_dim_inactive_panes;
        let frames = panes
            .iter()
            .map(|pane| {
                let is_selected = self.is_selected(pane);
                let is_dimmed = dims_pane(dim_inactive_panes, panes.len(), is_selected);
                (
                    pane.rect,
                    self.render_pane_frame(
                        pane,
                        is_selected,
                        is_dimmed,
                        can_close,
                        appearance,
                        app,
                    ),
                )
            })
            .collect();
        PaneFrames::new(layout.area, frames).finish()
    }

    /// A pane as Warp frames its panes: a header, and below it the pane's live output or its id
    /// while Herdr's pane has no contents to show yet. The header and border are drawn inside the
    /// frame Herdr's layout gives the pane, and the pane's cell grid is fitted to what they
    /// leave of it.
    fn render_pane_frame(
        &self,
        pane: &PresentedPane,
        is_selected: bool,
        is_dimmed: bool,
        can_close: bool,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let contents = app
            .has_singleton_model::<HerdrPaneContentsRegistry>()
            .then(|| HerdrPaneContentsRegistry::as_ref(app).contents(&pane.id))
            .flatten();
        let ownership = contents
            .as_ref()
            .and_then(|contents| contents.as_ref(app).ownership().cloned());
        let border = match &ownership {
            Some(PaneOwnership::Contested(_)) => theme.terminal_colors().normal.yellow.into(),
            _ => theme.split_pane_border_color(),
        };

        let mut layers = Stack::new();
        let has_screen = contents
            .as_ref()
            .is_some_and(|contents| contents.as_ref(app).terminal().is_some());
        if let Some(contents) = contents {
            layers.extend([PaneGridElement::new(
                contents,
                PaneGridStyle {
                    theme: theme.clone(),
                    font_family: appearance.monospace_font_family(),
                    font_size: appearance.monospace_font_size(),
                    line_height_ratio: appearance.ui_builder().line_height_ratio(),
                },
                PaneInputTarget {
                    pane: pane.id.clone(),
                    selected: is_selected,
                    window_id: self.window_id,
                    view_id: self.view_id,
                },
            )
            .finish()]);
        }
        if !has_screen {
            layers.extend([Self::render_pane_id(pane, appearance)]);
        }
        if let Some(notice) = ownership.as_ref().and_then(Self::pane_notice) {
            let notice = Text::new_inline(notice, appearance.ui_font_family(), DETAIL_FONT_SIZE)
                .with_color(theme.sub_text_color(theme.surface_1()).into())
                .finish();
            let badge = Container::new(notice)
                .with_horizontal_padding(ROW_HORIZONTAL_PADDING / 2.)
                .with_background(theme.surface_1())
                .finish();
            layers.extend([Align::new(badge).top_right().finish()]);
        }

        let mouse_states = self
            .pane_header_states
            .borrow_mut()
            .entry(pane.id.clone())
            .or_default()
            .clone();
        let frame = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(render_pane_header(
                pane,
                can_close,
                &mouse_states,
                appearance,
            ))
            .with_child(Expanded::new(1., layers.finish()).finish());
        let mut frame = Container::new(frame.finish())
            .with_border(Border::all(PANE_BORDER_WIDTH).with_border_fill(border));
        if is_dimmed {
            frame = frame.with_foreground_overlay(theme.inactive_pane_overlay());
        }
        frame.finish()
    }

    /// The marker a pane shows when warpr is not simply controlling it.
    fn pane_notice(ownership: &PaneOwnership) -> Option<String> {
        match ownership {
            PaneOwnership::Contested(Contest::Refused) => {
                Some("Another client controls this pane; showing it at Herdr's size".to_owned())
            }
            PaneOwnership::Contested(Contest::Evicted) => {
                Some("Another client took over this pane; showing it at Herdr's size".to_owned())
            }
            PaneOwnership::TookControl => {
                Some("Took control of this pane from another client".to_owned())
            }
            PaneOwnership::Exited => Some("Process exited".to_owned()),
            PaneOwnership::Unavailable(reason) => Some(format!("Pane stream ended: {reason}")),
            PaneOwnership::Owned | PaneOwnership::Released => None,
        }
    }

    fn render_pane_id(pane: &PresentedPane, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let id = Text::new_inline(
            pane.id.as_str().to_owned(),
            appearance.ui_font_family(),
            DETAIL_FONT_SIZE,
        )
        .with_color(theme.sub_text_color(theme.background()).into())
        .finish();
        Align::new(id).finish()
    }
}

impl HerdrWorkspaceView {
    /// The workspace's metadata tokens as Herdr's sidebar shows them, or nothing while it has none.
    fn render_metadata_tokens(
        workspace: &PresentedWorkspace,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        if workspace.tokens.is_empty() {
            return None;
        }
        let theme = appearance.theme();
        let text = workspace
            .tokens
            .iter()
            .map(|token| format!("{}: {}", token.name, token.value))
            .collect::<Vec<_>>()
            .join("  ");
        let text = Text::new_inline(text, appearance.ui_font_family(), DETAIL_FONT_SIZE)
            .with_color(theme.sub_text_color(theme.surface_1()).into())
            .finish();
        Some(
            Container::new(text)
                .with_horizontal_padding(ROW_HORIZONTAL_PADDING)
                .with_vertical_padding(ROW_VERTICAL_PADDING)
                .with_background(theme.surface_1())
                .finish(),
        )
    }
}

impl HerdrWorkspaceView {
    fn render_workspace(&self, app: &AppContext, appearance: &Appearance) -> Box<dyn Element> {
        let presented = self.presented_workspace(app);
        let Some((workspace, shown)) = presented.and_then(|workspace| {
            let shown = workspace.selected_tab(self.selected_tab.as_ref())?;
            Some((workspace, shown))
        }) else {
            return self.render_workspace_label(appearance);
        };

        let mut column = Flex::column();
        if let Some(tokens) = Self::render_metadata_tokens(workspace, appearance) {
            column.add_child(tokens);
        }
        column
            .with_child(Expanded::new(1., self.render_pane_frames(shown, appearance, app)).finish())
            .finish()
    }

    /// What to tell the user while the session is not connected, so that the workspaces shown
    /// are not mistaken for Herdr's current state.
    fn connection_notice(&self, app: &AppContext) -> Option<String> {
        if !app.has_singleton_model::<HerdrSessionModel>() {
            return None;
        }
        match &HerdrSessionModel::as_ref(app).presented().connection {
            ConnectionStatus::Reconnecting(failure) => Some(format!(
                "Lost the connection to Herdr, reconnecting: {failure}"
            )),
            ConnectionStatus::Disconnected(failure) => Some(format!(
                "Disconnected from Herdr, still trying to reconnect: {failure}"
            )),
            ConnectionStatus::Connecting
            | ConnectionStatus::Connected
            | ConnectionStatus::Failed(_)
            | ConnectionStatus::Refused(_) => None,
        }
    }

    fn render_connection_notice(notice: String, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let text = Text::new_inline(notice, appearance.ui_font_family(), DETAIL_FONT_SIZE)
            .with_color(theme.sub_text_color(theme.surface_1()).into())
            .finish();
        Container::new(text)
            .with_horizontal_padding(ROW_HORIZONTAL_PADDING)
            .with_vertical_padding(ROW_VERTICAL_PADDING)
            .with_background(theme.surface_1())
            .finish()
    }
}

impl HerdrWorkspaceView {
    /// Asks Herdr to create a Herdr tab in this workspace. Nothing is added here: the tab is
    /// shown, and active, once the session presents it. A refused request is shown in the view.
    fn create_tab(&mut self, ctx: &mut ViewContext<Self>) {
        let HerdrWorkspaceContents::Workspace(workspace) = &self.contents else {
            return;
        };
        let workspace_id = workspace.workspace_id.clone();
        let Some(target) = HerdrSessionModel::as_ref(ctx).session_target() else {
            self.request_error =
                Some("Could not create a tab: no Herdr session is connected".to_owned());
            ctx.notify();
            return;
        };
        ctx.spawn(
            async move { herdr_client::create_herdr_tab(&target, &workspace_id) },
            |me, result, ctx| {
                match result {
                    Ok(()) => {
                        me.request_error = None;
                        me.selected_tab = None;
                        me.selected_pane = None;
                        me.report_viewed_panes(ctx);
                        me.update_shown_panes(ctx);
                        ctx.emit(PaneEvent::HerdrTabChanged);
                    }
                    Err(failure) => {
                        log::error!("Could not create a Herdr tab: {failure}");
                        me.request_error = Some(format!("Could not create a tab: {failure}"));
                    }
                }
                ctx.notify();
            },
        );
    }

    /// Asks Herdr to close `pane`. Nothing is removed here: the pane leaves the view once the
    /// session stops presenting it. A refused request is shown in the view.
    fn close_pane(&mut self, pane: &PaneId, ctx: &mut ViewContext<Self>) {
        let Some(target) = HerdrSessionModel::as_ref(ctx).session_target() else {
            self.request_error =
                Some("Could not close the pane: no Herdr session is connected".to_owned());
            ctx.notify();
            return;
        };
        let pane = pane.clone();
        ctx.spawn(
            {
                let pane = pane.clone();
                async move { herdr_client::close_pane(&target, &pane) }
            },
            move |me, result, ctx| {
                match result {
                    Ok(()) => {
                        me.request_error = None;
                        if me.selected_pane.as_ref() == Some(&pane) {
                            me.selected_pane = None;
                            me.report_viewed_panes(ctx);
                            me.update_shown_panes(ctx);
                            me.sync_sidebar_details(ctx);
                        }
                    }
                    Err(failure) => {
                        log::error!(
                            "Could not close the Herdr pane {}: {failure}",
                            pane.as_str()
                        );
                        me.request_error = Some(format!("Could not close the pane: {failure}"));
                    }
                }
                ctx.notify();
            },
        );
    }
}

impl Entity for HerdrWorkspaceView {
    type Event = PaneEvent;
}

impl View for HerdrWorkspaceView {
    fn ui_name() -> &'static str {
        "HerdrWorkspaceView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let workspace = self.render_workspace(app, appearance);
        let Some(notice) = self
            .connection_notice(app)
            .or_else(|| self.request_error.clone())
        else {
            return workspace;
        };
        Flex::column()
            .with_child(Self::render_connection_notice(notice, appearance))
            .with_child(Expanded::new(1., workspace).finish())
            .finish()
    }
}

impl TypedActionView for HerdrWorkspaceView {
    type Action = HerdrWorkspaceViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            HerdrWorkspaceViewAction::CreateTab => self.create_tab(ctx),
            HerdrWorkspaceViewAction::ClosePane(pane) => self.close_pane(pane, ctx),
            HerdrWorkspaceViewAction::Paste(pane) => {
                let text = ctx
                    .clipboard()
                    .read()
                    .plain_text
                    .replace("\r\n", "\r")
                    .replace('\n', "\r");
                let contents = HerdrPaneContentsRegistry::as_ref(ctx).contents(pane);
                if let Some(contents) = contents.filter(|_| !text.is_empty()) {
                    contents.read(ctx, |contents, _| {
                        contents.stream().forward(PaneInput::Text(text));
                    });
                }
            }
            HerdrWorkspaceViewAction::SelectPane(pane) => {
                self.selected_pane = Some(pane.clone());
                self.report_viewed_panes(ctx);
                self.update_shown_panes(ctx);
                self.sync_sidebar_details(ctx);
                ctx.focus_self();
                ctx.notify();
            }
            HerdrWorkspaceViewAction::SelectTab(tab_id) => {
                self.selected_tab = Some(tab_id.clone());
                self.selected_pane = None;
                self.report_viewed_panes(ctx);
                self.update_shown_panes(ctx);
                self.sync_sidebar_details(ctx);
                ctx.emit(PaneEvent::HerdrTabChanged);
                ctx.notify();
            }
        }
    }
}

impl BackingView for HerdrWorkspaceView {
    type PaneHeaderOverflowMenuAction = ();
    type CustomAction = ();
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        _action: &Self::PaneHeaderOverflowMenuAction,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        self.show_panes(Vec::new(), ctx);
        ctx.emit(PaneEvent::Close);
    }

    fn focus_contents(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus_self();
        self.report_viewed_panes(ctx);
    }

    fn render_header_content(
        &self,
        _ctx: &view::HeaderRenderContext<'_>,
        _app: &AppContext,
    ) -> view::HeaderContent {
        view::HeaderContent::simple(self.contents.title().to_owned())
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}

#[cfg(all(test, unix))]
#[path = "workspace_view_tests.rs"]
mod tests;
