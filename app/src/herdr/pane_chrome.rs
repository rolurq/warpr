use herdr_client::{PaneLayout, PresentedAgent, PresentedPane};
use warpui::Element;
use warpui::elements::{
    CrossAxisAlignment, Empty, Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    ParentElement as _,
};

use super::workspace_view::HerdrWorkspaceViewAction;
use crate::appearance::Appearance;
use crate::pane_group::pane::view::StandardHeader;
use crate::pane_group::pane::view::header::components::{
    header_edge_min_width, render_close_button, render_standard_header_row,
};

/// The width of the border drawn around each pane.
pub(super) const PANE_BORDER_WIDTH: f32 = 1.;

/// The mouse state of one pane's header, kept across renders so that hovering and clicking work.
#[derive(Clone, Default)]
pub(super) struct PaneHeaderMouseStates {
    hover: MouseStateHandle,
    close_button: MouseStateHandle,
    click: MouseStateHandle,
}

/// The title of a pane's header: the name Herdr gives the pane, else the folder of its working
/// directory, else its id.
pub(super) fn pane_title(pane: &PresentedPane) -> String {
    if let Some(name) = &pane.name {
        return name.clone();
    }
    match &pane.cwd {
        Some(cwd) => cwd
            .file_name()
            .map_or_else(|| cwd.to_string_lossy(), |folder| folder.to_string_lossy())
            .into_owned(),
        None => pane.id.as_str().to_owned(),
    }
}

/// What Herdr says is running in a pane: the name of the agent it detects there, else the kind
/// of that agent, else the name given to the pane, else the title its program set. `None` when
/// Herdr tells none of these.
pub(super) fn pane_command(pane: &PresentedPane, agent: Option<&PresentedAgent>) -> Option<String> {
    agent
        .and_then(|agent| agent.name.as_ref().or(agent.kind.as_ref()))
        .or(pane.name.as_ref())
        .or(pane.title.as_ref())
        .cloned()
}

/// Whether the panes of `layout` can be closed. Herdr closes a Herdr tab along with its last
/// pane, so the last one is not offered; a zoomed tab still holds the panes its zoom hides.
pub(super) fn can_close_panes(layout: &PaneLayout) -> bool {
    layout.panes.len() > 1
}

/// Whether a pane is drawn dimmed: when the setting is on, it shares its view with other panes
/// and is not the selected one.
pub(super) fn dims_pane(setting_on: bool, shown_panes: usize, is_selected: bool) -> bool {
    setting_on && shown_panes > 1 && !is_selected
}

/// Warp's standard pane header for a Herdr pane: its title centered, and a close button while the
/// pointer is over the header. Clicking the header selects the pane.
pub(super) fn render_pane_header(
    pane: &PresentedPane,
    can_close: bool,
    mouse_states: &PaneHeaderMouseStates,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let mut header = StandardHeader::simple(pane_title(pane));
    header.options.hide_close_button = !can_close;
    let icon_color = appearance
        .theme()
        .sub_text_color(appearance.theme().background());
    let close = HerdrWorkspaceViewAction::ClosePane(pane.id.clone());
    let close_button_state = mouse_states.close_button.clone();

    let row = render_standard_header_row(
        header,
        appearance,
        mouse_states.hover.clone(),
        Empty::new().finish(),
        0.,
        |is_hovered, _, options| {
            let has_close_button = !options.hide_close_button;
            let mut controls = Flex::row()
                .with_main_axis_alignment(MainAxisAlignment::End)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Max);
            if has_close_button && is_hovered {
                controls.add_child(render_close_button(
                    appearance,
                    close_button_state,
                    Some(icon_color),
                    None,
                    close,
                ));
            }
            (controls, header_edge_min_width(u32::from(has_close_button)))
        },
    );

    let select = HerdrWorkspaceViewAction::SelectPane(pane.id.clone());
    Hoverable::new(mouse_states.click.clone(), |_| row)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(select.clone()))
        .finish()
}

#[cfg(test)]
#[path = "pane_chrome_tests.rs"]
mod tests;
