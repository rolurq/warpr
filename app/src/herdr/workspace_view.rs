use warpui::elements::{
    Align, CrossAxisAlignment, Flex, MainAxisAlignment, ParentElement as _, Text,
};
use warpui::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity as _, TypedActionView, View,
    ViewContext,
};

use crate::app_state::HerdrWorkspaceSnapshot;
use crate::appearance::Appearance;
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::pane::view;
use crate::pane_group::{BackingView, PaneConfiguration, PaneEvent};

const LABEL_FONT_SIZE: f32 = 20.;
const DETAIL_FONT_SIZE: f32 = 12.;

/// The contents of the pane that stands for a Herdr workspace in its top-level tab.
pub struct HerdrWorkspaceView {
    workspace: HerdrWorkspaceSnapshot,
    pane_configuration: ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,
}

impl HerdrWorkspaceView {
    pub fn new(workspace: HerdrWorkspaceSnapshot, ctx: &mut ViewContext<Self>) -> Self {
        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new(&workspace.label));
        Self {
            workspace,
            pane_configuration,
            focus_handle: None,
        }
    }

    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    pub fn snapshot(&self) -> HerdrWorkspaceSnapshot {
        self.workspace.clone()
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
        let theme = appearance.theme();
        let font_family = appearance.ui_font_family();
        let label = Text::new_inline(self.workspace.label.clone(), font_family, LABEL_FONT_SIZE)
            .with_color(theme.main_text_color(theme.background()).into())
            .finish();
        let detail = Text::new_inline(
            format!("Herdr workspace {}", self.workspace.workspace_id.as_str()),
            font_family,
            DETAIL_FONT_SIZE,
        )
        .with_color(theme.sub_text_color(theme.background()).into())
        .finish();

        Align::new(
            Flex::column()
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(label)
                .with_child(detail)
                .finish(),
        )
        .finish()
    }
}

impl TypedActionView for HerdrWorkspaceView {
    type Action = ();

    fn handle_action(&mut self, _action: &Self::Action, _ctx: &mut ViewContext<Self>) {}
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
        view::HeaderContent::simple(self.workspace.label.clone())
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}
