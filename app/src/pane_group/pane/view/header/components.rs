use warp_core::ui::icons::ICON_DIMENSIONS;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    Align, Clipped, ConstrainedBox, Container, CrossAxisAlignment, Flex, Hoverable,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, SavePosition, Shrinkable,
    Text,
};
use warpui::text_layout::ClipConfig;
use warpui::ui_components::components::UiComponent;
use warpui::{Action, Element};

use super::super::header_content::{HeaderRenderContext, StandardHeader, StandardHeaderOptions};
use super::{ActionPayload, PANE_HEADER_HEIGHT, PaneHeaderAction};
use crate::appearance::Appearance;
use crate::ui_components::buttons::{icon_button, icon_button_with_color};
use crate::ui_components::icons::Icon;

/// Horizontal padding applied inside each edge column of the three-column header.
pub const HEADER_EDGE_PADDING: f32 = 4.;

fn build_icon_button(
    appearance: &Appearance,
    icon: Icon,
    mouse_state: MouseStateHandle,
    icon_color: Option<Fill>,
) -> Hoverable {
    if let Some(color) = icon_color {
        icon_button_with_color(appearance, icon, false, mouse_state, color)
    } else {
        icon_button(appearance, icon, false, mouse_state)
    }
    .build()
}

fn apply_size_constraint(element: Box<dyn Element>, size: Option<f32>) -> Box<dyn Element> {
    if let Some(size) = size {
        ConstrainedBox::new(element)
            .with_width(size)
            .with_height(size)
            .finish()
    } else {
        element
    }
}

/// Renders the standard pane close button, dispatching `action` when it is clicked.
pub fn render_close_button<T: Action + Clone>(
    appearance: &Appearance,
    mouse_state: MouseStateHandle,
    icon_color: Option<Fill>,
    button_size: Option<f32>,
    action: T,
) -> Box<dyn Element> {
    let button = build_icon_button(appearance, Icon::X, mouse_state, icon_color)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()));
    apply_size_constraint(button.finish(), button_size)
}

/// Renders the standard pane close button to dispatch the close action.
pub fn render_pane_close_button<A: ActionPayload, B: ActionPayload>(
    appearance: &Appearance,
    mouse_state: MouseStateHandle,
    icon_color: Option<Fill>,
    button_size: Option<f32>,
) -> Box<dyn Element> {
    render_close_button(
        appearance,
        mouse_state,
        icon_color,
        button_size,
        PaneHeaderAction::<A, B>::Close,
    )
}

/// Renders the standard pane overflow menu button to dispatch the pane overflow menu action.
pub fn render_pane_overflow_button<A: ActionPayload, B: ActionPayload>(
    appearance: &Appearance,
    mouse_state: MouseStateHandle,
    position_id: &str,
    icon_color: Option<Fill>,
    button_size: Option<f32>,
) -> Box<dyn Element> {
    let button = build_icon_button(appearance, Icon::DotsVertical, mouse_state, icon_color)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(PaneHeaderAction::<A, B>::OpenOverflowMenu)
        });
    let button = apply_size_constraint(button.finish(), button_size);
    SavePosition::new(button, position_id).finish()
}

/// Renders a row containing the standard pane overflow and close buttons.
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub fn render_pane_header_buttons<A: ActionPayload, B: ActionPayload>(
    header_ctx: &HeaderRenderContext<'_>,
    appearance: &Appearance,
    show_close_button: bool,
    icon_color: Option<Fill>,
    button_size: Option<f32>,
) -> Box<dyn Element> {
    let mut row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Min);

    if header_ctx.has_overflow_items {
        row.add_child(render_pane_overflow_button::<A, B>(
            appearance,
            header_ctx.overflow_button_mouse_state.clone(),
            &header_ctx.overflow_button_position_id,
            icon_color,
            button_size,
        ));
    }

    if show_close_button {
        row.add_child(render_pane_close_button::<A, B>(
            appearance,
            header_ctx.close_button_mouse_state.clone(),
            icon_color,
            button_size,
        ));
    }

    row.finish()
}

/// Renders a title text element with the standard pane header font, color, and clipping.
pub fn render_pane_header_title_text(
    title: impl Into<std::borrow::Cow<'static, str>>,
    appearance: &Appearance,
    clip_config: ClipConfig,
) -> Box<dyn Element> {
    let font_size = appearance.ui_font_size();
    let font_color = appearance
        .theme()
        .sub_text_color(appearance.theme().background());
    Text::new_inline(title, appearance.ui_font_family(), font_size)
        .with_color(font_color.into())
        .with_clip(clip_config)
        .finish()
}

/// Estimates the minimum width needed for a header edge column containing
/// `icon_button_count` standard icon buttons, accounting for the edge
/// column's internal padding.
pub fn header_edge_min_width(icon_button_count: u32) -> f32 {
    icon_button_count as f32 * ICON_DIMENSIONS + HEADER_EDGE_PADDING
}

/// Width constraints applied to both the left and right edge columns in
/// [`render_three_column_header`].  Giving both edges the same min/max
/// keeps the center title visually centered regardless of how much content
/// each side actually contains.
pub struct CenteredHeaderEdgeWidth {
    /// Minimum width — typically the width of the always-visible buttons
    /// so they are never clipped.
    pub min: f32,
    /// Maximum width — caps how far the edge columns can grow so they
    /// don't eat into the title's space.
    pub max: f32,
}

/// Renders a 3-column header layout: `[left] [title] [right]`.
///
/// The `left` and `right` edge columns share equal min/max width constraints
/// so the title stays centered. Each edge gets inner padding (left or right).
/// `extra_left_inset` adds additional left padding inside the left column
/// (e.g. to make room for a floating overlay button) without affecting centering.
pub fn render_three_column_header(
    left: Box<dyn Element>,
    title: Box<dyn Element>,
    right: Box<dyn Element>,
    edge_width: CenteredHeaderEdgeWidth,
    extra_left_inset: f32,
    is_pane_dragging: bool,
) -> Box<dyn Element> {
    let main_axis_size = if is_pane_dragging {
        MainAxisSize::Min
    } else {
        MainAxisSize::Max
    };

    let left_constrained = ConstrainedBox::new(
        Container::new(left)
            .with_padding_left(HEADER_EDGE_PADDING + extra_left_inset)
            .finish(),
    )
    .with_min_width(edge_width.min)
    .with_max_width(edge_width.max)
    .finish();

    let right_constrained = ConstrainedBox::new(
        Container::new(right)
            .with_padding_right(HEADER_EDGE_PADDING)
            .finish(),
    )
    .with_min_width(edge_width.min)
    .with_max_width(edge_width.max)
    .finish();

    let mut center_row = Flex::row()
        .with_main_axis_alignment(MainAxisAlignment::Center)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(main_axis_size);
    center_row.add_child(if is_pane_dragging {
        title
    } else {
        Shrinkable::new(1., Clipped::new(title).finish()).finish()
    });
    let center = Align::new(center_row.finish()).finish();

    let mut row = Flex::row()
        .with_main_axis_size(main_axis_size)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch);

    row.add_child(if is_pane_dragging {
        left_constrained
    } else {
        Shrinkable::new(1., left_constrained).finish()
    });
    row.add_child(if is_pane_dragging {
        center
    } else {
        Shrinkable::new(1., center).finish()
    });
    row.add_child(right_constrained);

    row.finish()
}

/// Lays out `header` as Warp's standard pane header: `[left] [title] [controls]`, with the edge
/// columns sharing one width so the title stays centered.
///
/// `controls` builds the right-hand column from whether the pointer is over the header, the
/// header's `left_of_overflow` element and its options, and returns it with the minimum width
/// of the buttons that column always has room for.
pub(crate) fn render_standard_header_row(
    header: StandardHeader,
    appearance: &Appearance,
    hover_handle: MouseStateHandle,
    left: Box<dyn Element>,
    left_inset: f32,
    controls: impl FnOnce(bool, Option<Box<dyn Element>>, &StandardHeaderOptions) -> (Flex, f32),
) -> Box<dyn Element> {
    let StandardHeader {
        title,
        title_secondary,
        title_style,
        title_clip_config,
        title_max_width,
        left_of_title,
        right_of_title,
        left_of_overflow,
        options,
    } = header;

    Hoverable::new(hover_handle, |hover_state| {
        let (right_justified_row, min_right_width) =
            controls(hover_state.is_hovered(), left_of_overflow, &options);

        // Build the center row with title and optional customization elements.
        let mut center_row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max);

        if let Some(left_element) = left_of_title {
            center_row.add_child(Container::new(left_element).with_margin_right(4.).finish());
        }

        let font_size = appearance.ui_font_size();
        let font_color = appearance
            .theme()
            .sub_text_color(appearance.theme().background());

        // Build title row with primary title and optional secondary title.
        let mut title_row = Flex::row();
        let title_text = Text::new_inline(title, appearance.ui_font_family(), font_size)
            .with_color(font_color.into())
            .with_style(title_style.unwrap_or_default())
            .with_clip(title_clip_config)
            .finish();
        title_row.add_child(Shrinkable::new(1., title_text).finish());

        if let Some(secondary) = title_secondary
            && !secondary.is_empty()
        {
            let secondary_text =
                Text::new_inline(secondary, appearance.ui_font_family(), font_size)
                    .with_color(font_color.into())
                    .finish();
            title_row.add_child(secondary_text);
        }

        // If a max width is set, constrain the title to that width.
        let title_element = if let Some(max_width) = title_max_width {
            Shrinkable::new(
                1.,
                ConstrainedBox::new(title_row.finish())
                    .with_max_width(max_width)
                    .finish(),
            )
            .finish()
        } else {
            Shrinkable::new(1., title_row.finish()).finish()
        };
        center_row.add_child(title_element);

        if let Some(right_element) = right_of_title {
            center_row.add_child(Container::new(right_element).with_margin_left(4.).finish());
        }

        // Build the left column.
        let mut left_justified_row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min);
        left_justified_row.add_child(left);

        let left_justified_container =
            Container::new(left_justified_row.finish()).with_padding_left(4. + left_inset);
        let right_justified_container =
            Container::new(right_justified_row.finish()).with_padding_right(4.);

        let edge_width = options.control_container_width();
        let left_constrained = ConstrainedBox::new(left_justified_container.finish())
            .with_min_width(min_right_width)
            .with_max_width(edge_width)
            .finish();
        let right_constrained = ConstrainedBox::new(right_justified_container.finish())
            .with_min_width(min_right_width)
            .with_max_width(edge_width)
            .finish();

        // Build the complete 3-column layout.
        let mut row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        row.add_child(left_constrained);
        // Wrap center_row in Align to vertically center within the stretched space.
        row.add_child(Shrinkable::new(1., Align::new(center_row.finish()).finish()).finish());
        row.add_child(right_constrained);

        Container::new(
            Clipped::new(
                ConstrainedBox::new(row.finish())
                    .with_height(PANE_HEADER_HEIGHT)
                    .finish(),
            )
            .finish(),
        )
        .finish()
    })
    .finish()
}
