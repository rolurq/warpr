use std::cell::RefCell;
use std::collections::HashMap;

use herdr_client::{AgentState, ConnectionStatus, PaneId, PresentedAgent, PresentedSession};
use warpui::elements::{
    ClippedScrollStateHandle, ClippedScrollable, Container, CrossAxisAlignment, Element, Expanded,
    Fill, Flex, Hoverable, MouseStateHandle, ParentElement as _, ScrollbarWidth, Text,
};
use warpui::platform::Cursor;
use warpui::{AppContext, Entity, SingletonEntity as _, TypedActionView, View, ViewContext};

use super::{HerdrSessionEvent, HerdrSessionModel};
use crate::appearance::Appearance;

const TITLE_FONT_SIZE: f32 = 14.;
const DETAIL_FONT_SIZE: f32 = 12.;
const ROW_HORIZONTAL_PADDING: f32 = 16.;
const ROW_VERTICAL_PADDING: f32 = 8.;
const NO_KIND: &str = "unknown kind";

#[derive(Clone, Debug, PartialEq)]
pub enum HerdrAgentPanelAction {
    /// Takes the user to the pane of the agent in this pane. Only warpr's own view changes:
    /// Herdr's focus is left alone, so other Herdr clients see no change.
    Jump(PaneId),
}

/// What one agent's row says.
#[derive(Clone, Debug, PartialEq, Eq)]
struct AgentRow {
    pane: PaneId,
    /// The agent's live name, else what Herdr recognized it as, else its pane.
    title: String,
    state: AgentState,
    /// The kind, then the workspace, Herdr tab and pane.
    detail: String,
}

impl AgentRow {
    fn of(agent: &PresentedAgent, session: &PresentedSession) -> Self {
        let title = agent
            .name
            .as_deref()
            .or(agent.kind.as_deref())
            .unwrap_or(agent.pane.as_str())
            .to_owned();
        let (workspace, herdr_tab) = match agent.location(session) {
            Some(location) => (
                location.workspace.label.clone(),
                location.herdr_tab.label.clone(),
            ),
            None => (
                agent.workspace.as_str().to_owned(),
                agent.herdr_tab.as_str().to_owned(),
            ),
        };
        let kind = agent.kind.as_deref().unwrap_or(NO_KIND);
        Self {
            pane: agent.pane.clone(),
            title,
            state: agent.state,
            detail: format!(
                "{kind} · {workspace} › {herdr_tab} › {}",
                agent.pane.as_str()
            ),
        }
    }
}

/// Lists every agent Herdr recognizes with its state and where it runs, and takes the user to an
/// agent's pane when its row is clicked.
pub struct HerdrAgentPanelView {
    scroll_state: ClippedScrollStateHandle,
    /// Filled while rendering, because Herdr decides which agents exist and `render` only has
    /// shared access to the view.
    row_mouse_states: RefCell<HashMap<PaneId, MouseStateHandle>>,
}

impl HerdrAgentPanelView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(
            &HerdrSessionModel::handle(ctx),
            |_, _, event, ctx| match event {
                HerdrSessionEvent::Presented | HerdrSessionEvent::AgentsPresented => ctx.notify(),
                HerdrSessionEvent::SessionsListed | HerdrSessionEvent::JumpedToAgent(_) => {}
            },
        );
        Self {
            scroll_state: ClippedScrollStateHandle::default(),
            row_mouse_states: RefCell::default(),
        }
    }

    fn rows(app: &AppContext) -> Vec<AgentRow> {
        let model = HerdrSessionModel::as_ref(app);
        model
            .agents()
            .agents
            .iter()
            .map(|agent| AgentRow::of(agent, model.presented()))
            .collect()
    }

    /// What to say instead of, or next to, the rows when the agents are not simply connected.
    fn notice(app: &AppContext) -> Option<String> {
        match &HerdrSessionModel::as_ref(app).agents().connection {
            ConnectionStatus::Connecting => Some("Connecting to Herdr…".to_owned()),
            ConnectionStatus::Connected => None,
            ConnectionStatus::Reconnecting(failure) => Some(format!(
                "Lost the connection to Herdr, reconnecting: {failure}"
            )),
            ConnectionStatus::Disconnected(failure) => Some(format!(
                "Disconnected from Herdr, still trying to reconnect: {failure}"
            )),
            ConnectionStatus::Failed(failure) => Some(failure.to_string()),
            ConnectionStatus::Refused(refusal) => Some(refusal.to_string()),
        }
    }

    fn render_state(state: AgentState, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let color = match state {
            AgentState::Blocked => theme.ui_error_color(),
            AgentState::Working => theme.accent().into(),
            AgentState::Done => theme.ui_green_color(),
            AgentState::Idle | AgentState::Unknown => {
                theme.sub_text_color(theme.background()).into()
            }
        };
        Text::new_inline(
            state.as_str().to_owned(),
            appearance.ui_font_family(),
            DETAIL_FONT_SIZE,
        )
        .with_color(color)
        .finish()
    }

    fn render_row(&self, row: AgentRow, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let mouse_state = self
            .row_mouse_states
            .borrow_mut()
            .entry(row.pane.clone())
            .or_default()
            .clone();
        let pane = row.pane.clone();
        Hoverable::new(mouse_state, |state| {
            let background = if state.is_hovered() {
                theme.surface_2()
            } else {
                theme.background()
            };
            let title = Text::new_inline(
                row.title.clone(),
                appearance.ui_font_family(),
                TITLE_FONT_SIZE,
            )
            .with_color(theme.main_text_color(background).into())
            .finish();
            let heading = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(Expanded::new(1., title).finish())
                .with_child(Self::render_state(row.state, appearance))
                .finish();
            let detail = Text::new_inline(
                row.detail.clone(),
                appearance.ui_font_family(),
                DETAIL_FONT_SIZE,
            )
            .with_color(theme.sub_text_color(background).into())
            .finish();
            Container::new(
                Flex::column()
                    .with_child(heading)
                    .with_child(detail)
                    .finish(),
            )
            .with_horizontal_padding(ROW_HORIZONTAL_PADDING)
            .with_vertical_padding(ROW_VERTICAL_PADDING)
            .with_background(background)
            .finish()
        })
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(HerdrAgentPanelAction::Jump(pane.clone()));
        })
        .with_cursor(Cursor::PointingHand)
        .finish()
    }

    fn render_message(message: String, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let text = Text::new(message, appearance.ui_font_family(), TITLE_FONT_SIZE)
            .with_color(theme.sub_text_color(theme.background()).into())
            .finish();
        Container::new(text)
            .with_horizontal_padding(ROW_HORIZONTAL_PADDING)
            .with_vertical_padding(ROW_VERTICAL_PADDING)
            .finish()
    }
}

impl Entity for HerdrAgentPanelView {
    type Event = ();
}

impl View for HerdrAgentPanelView {
    fn ui_name() -> &'static str {
        "HerdrAgentPanelView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let rows = Self::rows(app);
        let notice = Self::notice(app);

        let mut list = Flex::column();
        if let Some(notice) = notice {
            list.add_child(Self::render_message(notice, appearance));
        }
        if rows.is_empty()
            && HerdrSessionModel::as_ref(app).agents().connection == ConnectionStatus::Connected
        {
            list.add_child(Self::render_message(
                "Herdr recognizes no agents.".to_owned(),
                appearance,
            ));
        }
        for row in rows {
            list.add_child(self.render_row(row, appearance));
        }

        ClippedScrollable::vertical(
            self.scroll_state.clone(),
            list.finish(),
            ScrollbarWidth::Auto,
            theme.disabled_text_color(theme.background()).into(),
            theme.main_text_color(theme.background()).into(),
            Fill::None,
        )
        .finish()
    }
}

impl TypedActionView for HerdrAgentPanelView {
    type Action = HerdrAgentPanelAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            HerdrAgentPanelAction::Jump(pane) => {
                HerdrSessionModel::handle(ctx).update(ctx, |model, ctx| {
                    model.jump_to_agent(pane, ctx);
                });
            }
        }
    }
}

#[cfg(all(test, unix))]
#[path = "agent_panel_tests.rs"]
mod tests;
