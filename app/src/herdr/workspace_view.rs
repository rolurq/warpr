use std::cell::RefCell;
use std::collections::HashMap;

use herdr_client::{
    AgentJump, ConnectionStatus, Contest, HerdrTabId, PaneId, PaneInput, PaneOwnership,
    PresentedHerdrTab, PresentedPane, PresentedWorkspace,
};
use warpui::elements::{
    Align, Border, ConstrainedBox, Container, CrossAxisAlignment, Expanded, Flex, Hoverable,
    MainAxisAlignment, MouseStateHandle, ParentElement as _, Stack, Text,
};
use warpui::platform::Cursor;
use warpui::{
    AppContext, Element, Entity, EntityId, ModelHandle, SingletonEntity as _, TypedActionView,
    View, ViewContext, WindowId,
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

const LABEL_FONT_SIZE: f32 = 20.;
const DETAIL_FONT_SIZE: f32 = 12.;
const TAB_FONT_SIZE: f32 = 13.;
const TAB_HORIZONTAL_PADDING: f32 = 12.;
const TAB_VERTICAL_PADDING: f32 = 6.;
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
    /// Filled while rendering, because Herdr decides which tabs exist and `render` only has
    /// shared access to the view.
    tab_mouse_states: RefCell<HashMap<HerdrTabId, MouseStateHandle>>,
    new_tab_mouse_state: MouseStateHandle,
    /// Why the last request to create a Herdr tab was refused, until the next one succeeds.
    creation_error: Option<String>,
    pane_configuration: ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,
    window_id: WindowId,
    view_id: EntityId,
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
        let mut view = Self {
            contents,
            selected_tab: None,
            selected_pane: None,
            tab_mouse_states: RefCell::default(),
            new_tab_mouse_state: MouseStateHandle::default(),
            creation_error: None,
            pane_configuration,
            focus_handle: None,
            window_id: ctx.window_id(),
            view_id: ctx.view_id(),
        };
        view.update_shown_panes(ctx);
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
        self.update_shown_panes(ctx);
        ctx.notify();
    }

    /// Tells the pane registry which panes this view shows: those of the Herdr tab on screen.
    fn update_shown_panes(&mut self, ctx: &mut ViewContext<Self>) {
        let shown = self.shown_panes(ctx);
        self.show_panes(shown, ctx);
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

    fn render_tab_strip(
        &self,
        workspace: &PresentedWorkspace,
        shown: &PresentedHerdrTab,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let mut mouse_states = self.tab_mouse_states.borrow_mut();
        let tabs = workspace.herdr_tabs.iter().map(|tab| {
            let is_shown = tab.id == shown.id;
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
                ctx.dispatch_typed_action(HerdrWorkspaceViewAction::SelectTab(tab_id.clone()));
            })
            .with_cursor(Cursor::PointingHand)
            .finish()
        });
        Container::new(
            Flex::row()
                .with_children(tabs)
                .with_child(self.render_new_tab_button(appearance))
                .finish(),
        )
        .with_background(theme.background())
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
        let frames = layout
            .frames()
            .into_iter()
            .map(|pane| {
                let is_selected = match &self.selected_pane {
                    Some(selected) => *selected == pane.id,
                    None => pane.focused,
                };
                (
                    pane.rect,
                    self.render_pane_frame(&pane, is_selected, appearance, app),
                )
            })
            .collect();
        PaneFrames::new(layout.area, frames).finish()
    }

    /// A pane's live output, or its id while Herdr's pane has no contents to show yet.
    fn render_pane_frame(
        &self,
        pane: &PresentedPane,
        is_selected: bool,
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
        let border = match (&ownership, is_selected) {
            (Some(PaneOwnership::Contested(_)), _) => theme.terminal_colors().normal.yellow.into(),
            (_, true) => theme.accent(),
            (_, false) => theme.outline(),
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
                .with_horizontal_padding(TAB_HORIZONTAL_PADDING / 2.)
                .with_background(theme.surface_1())
                .finish();
            layers.extend([Align::new(badge).top_right().finish()]);
        }
        Container::new(layers.finish())
            .with_border(Border::all(1.).with_border_fill(border))
            .finish()
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
                .with_horizontal_padding(TAB_HORIZONTAL_PADDING)
                .with_vertical_padding(TAB_VERTICAL_PADDING)
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
            .with_child(self.render_tab_strip(workspace, shown, appearance))
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
            .with_horizontal_padding(TAB_HORIZONTAL_PADDING)
            .with_vertical_padding(TAB_VERTICAL_PADDING)
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
            self.creation_error =
                Some("Could not create a tab: no Herdr session is connected".to_owned());
            ctx.notify();
            return;
        };
        ctx.spawn(
            async move { herdr_client::create_herdr_tab(&target, &workspace_id) },
            |me, result, ctx| {
                match result {
                    Ok(()) => {
                        me.creation_error = None;
                        me.selected_tab = None;
                        me.selected_pane = None;
                        me.report_viewed_panes(ctx);
                        me.update_shown_panes(ctx);
                    }
                    Err(failure) => {
                        log::error!("Could not create a Herdr tab: {failure}");
                        me.creation_error = Some(format!("Could not create a tab: {failure}"));
                    }
                }
                ctx.notify();
            },
        );
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
            ctx.dispatch_typed_action(HerdrWorkspaceViewAction::CreateTab);
        })
        .with_cursor(Cursor::PointingHand)
        .finish()
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
            .or_else(|| self.creation_error.clone())
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
                ctx.focus_self();
                ctx.notify();
            }
            HerdrWorkspaceViewAction::SelectTab(tab_id) => {
                self.selected_tab = Some(tab_id.clone());
                self.selected_pane = None;
                self.report_viewed_panes(ctx);
                self.update_shown_panes(ctx);
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
