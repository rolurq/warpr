use std::cell::RefCell;
use std::collections::HashMap;

use herdr_client::{
    ConnectionStatus, HerdrTabId, PresentedHerdrTab, PresentedPane, PresentedWorkspace,
};
use warpui::elements::{
    Align, Border, ConstrainedBox, Container, CrossAxisAlignment, Expanded, Flex, Hoverable,
    MainAxisAlignment, MouseStateHandle, ParentElement as _, Text,
};
use warpui::platform::Cursor;
use warpui::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity as _, TypedActionView, View,
    ViewContext,
};

use super::pane_frames::PaneFrames;
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
}

/// The contents of the pane that stands for a Herdr workspace in its top-level tab, or for the
/// session when warpr refuses to connect to it.
pub struct HerdrWorkspaceView {
    contents: HerdrWorkspaceContents,
    /// The Herdr tab the user picked here, if any; Herdr's active tab is shown until then.
    selected_tab: Option<HerdrTabId>,
    /// Filled while rendering, because Herdr decides which tabs exist and `render` only has
    /// shared access to the view.
    tab_mouse_states: RefCell<HashMap<HerdrTabId, MouseStateHandle>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,
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
                    HerdrSessionEvent::SessionsListed => {}
                },
            );
        }
        Self {
            contents,
            selected_tab: None,
            tab_mouse_states: RefCell::default(),
            pane_configuration,
            focus_handle: None,
        }
    }

    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    pub fn snapshot(&self) -> HerdrWorkspaceContents {
        self.contents.clone()
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
        ctx.notify();
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
        Container::new(Flex::row().with_children(tabs).finish())
            .with_background(theme.background())
            .finish()
    }

    fn render_pane_frames(
        &self,
        tab: &PresentedHerdrTab,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let Some(layout) = &tab.layout else {
            return Align::new(self.render_workspace_label(appearance)).finish();
        };
        let frames = layout
            .frames()
            .into_iter()
            .map(|pane| (pane.rect, Self::render_pane_frame(&pane, appearance)))
            .collect();
        PaneFrames::new(layout.area, frames).finish()
    }

    /// An empty placeholder: pane contents arrive with the live output of a later change.
    fn render_pane_frame(pane: &PresentedPane, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let border = if pane.focused {
            theme.accent()
        } else {
            theme.outline()
        };
        let id = Text::new_inline(
            pane.id.as_str().to_owned(),
            appearance.ui_font_family(),
            DETAIL_FONT_SIZE,
        )
        .with_color(theme.sub_text_color(theme.background()).into())
        .finish();
        Container::new(Align::new(id).finish())
            .with_border(Border::all(1.).with_border_fill(border))
            .finish()
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
            .with_child(Expanded::new(1., self.render_pane_frames(shown, appearance)).finish())
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
        let Some(notice) = self.connection_notice(app) else {
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
            HerdrWorkspaceViewAction::SelectTab(tab_id) => {
                self.selected_tab = Some(tab_id.clone());
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
        ctx.emit(PaneEvent::Close);
    }

    fn focus_contents(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus_self();
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
