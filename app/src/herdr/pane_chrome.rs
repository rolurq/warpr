use herdr_client::{PaneLayout, PresentedAgent, PresentedPane};
use warpui::Element;
use warpui::elements::{
    CrossAxisAlignment, Empty, Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    ParentElement as _,
};
use warpui::ui_components::components::UiComponent as _;

use super::workspace_view::HerdrWorkspaceViewAction;
use crate::appearance::Appearance;
use crate::pane_group::pane::view::StandardHeader;
use crate::pane_group::pane::view::header::components::{
    header_edge_min_width, render_close_button, render_standard_header_row,
};
use crate::ui_components::buttons::icon_button_with_color;
use crate::ui_components::icons::Icon;

/// The width of the border drawn around each pane.
pub(super) const PANE_BORDER_WIDTH: f32 = 1.;

/// The mouse state of one pane's header, kept across renders so that hovering and clicking work.
#[derive(Clone, Default)]
pub(super) struct PaneHeaderMouseStates {
    hover: MouseStateHandle,
    zoom_button: MouseStateHandle,
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

/// What a pane header's zoom control does when clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ZoomControl {
    Zoom,
    Unzoom,
}

/// The zoom control the panes of `layout` offer: none while the Herdr tab holds one pane, which
/// Herdr does not zoom, and an unzoom control while the tab is zoomed.
pub(super) fn zoom_control(layout: &PaneLayout) -> Option<ZoomControl> {
    if layout.panes.len() < 2 {
        return None;
    }
    Some(if layout.zoomed {
        ZoomControl::Unzoom
    } else {
        ZoomControl::Zoom
    })
}

/// Whether a pane is drawn dimmed: when the setting is on, it shares its view with other panes
/// and is not the selected one.
pub(super) fn dims_pane(setting_on: bool, shown_panes: usize, is_selected: bool) -> bool {
    setting_on && shown_panes > 1 && !is_selected
}

/// Warp's standard pane header for a Herdr pane: its title centered, and the zoom control and a
/// close button while the pointer is over the header. Clicking the header selects the pane.
pub(super) fn render_pane_header(
    pane: &PresentedPane,
    can_close: bool,
    zoom: Option<ZoomControl>,
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
    let toggle_zoom = HerdrWorkspaceViewAction::ToggleZoom(pane.id.clone());
    let zoom_button_state = mouse_states.zoom_button.clone();

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
            if let Some(zoom) = zoom.filter(|_| is_hovered) {
                let icon = match zoom {
                    ZoomControl::Zoom => Icon::Maximize,
                    ZoomControl::Unzoom => Icon::Minimize,
                };
                controls.add_child(
                    icon_button_with_color(appearance, icon, false, zoom_button_state, icon_color)
                        .build()
                        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(toggle_zoom.clone()))
                        .finish(),
                );
            }
            if has_close_button && is_hovered {
                controls.add_child(render_close_button(
                    appearance,
                    close_button_state,
                    Some(icon_color),
                    None,
                    close,
                ));
            }
            let buttons = u32::from(zoom.is_some()) + u32::from(has_close_button);
            (controls, header_edge_min_width(buttons))
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
