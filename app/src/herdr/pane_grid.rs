use std::collections::HashMap;
use std::ops::RangeInclusive;

use herdr_client::{MouseAction, PaneId, PaneInput, PaneOwnership, PaneSize};
use warp_core::ui::theme::WarpTheme;
use warpui::elements::{Border, Point};
use warpui::event::DispatchedEvent;
use warpui::fonts::{FamilyId, Properties};
use warpui::geometry::rect::RectF;
use warpui::geometry::vector::{Vector2F, vec2f};
use warpui::units::IntoPixels;
use warpui::{
    AfterLayoutContext, AppContext, ClipBounds, Element, EntityId, Event, EventContext,
    LayoutContext, ModelHandle, PaintContext, SizeConstraint, WindowId,
};

use super::pane_contents::HerdrPaneContents;
use super::pane_input::{cell_at, is_paste, keystroke_input, scroll_lines};
use super::pane_terminal::PaneCursor;
use super::workspace_view::HerdrWorkspaceViewAction;
use crate::settings::EnforceMinimumContrast;
use crate::terminal::color::{self, OverrideList};
use crate::terminal::grid_renderer::{CellGlyphCache, render_grid};
use crate::terminal::grid_size_util::grid_cell_dimensions;
use crate::terminal::model::ansi::CursorShape;
use crate::terminal::model::grid::RespectDisplayedOutput;
use crate::terminal::model::index::Point as GridPoint;
use crate::terminal::model::secrets::ObfuscateSecrets;

/// The cursor's thickness for beam and underline shapes, relative to a cell's width.
const CURSOR_THICKNESS_SCALE_FACTOR: f32 = 0.15;

/// How the cells of a pane's screen are drawn.
pub struct PaneGridStyle {
    pub theme: WarpTheme,
    pub font_family: FamilyId,
    pub font_size: f32,
    pub line_height_ratio: f32,
}

/// The pane and view a grid forwards input for.
pub struct PaneInputTarget {
    pub pane: PaneId,
    /// Whether this is the pane that has the focus of warpr's view of the workspace.
    pub selected: bool,
    pub window_id: WindowId,
    pub view_id: EntityId,
}

/// Paints the screen of a pane and tells its stream how many cells the frame holds.
///
/// A pane warpr controls is drawn at the font's own cell size and left padded where the frame is
/// not a whole number of cells. A contested pane is watched at Herdr's grid, which need not fit
/// the frame, so it is drawn at the font's size when it fits and shrunk with its font until it
/// does when it does not; what is left of the frame stays blank.
///
/// Keys, typed text, scrolling and left-button presses go to the pane's program when this is the
/// selected pane of the focused view. A press anywhere else only selects the pane.
pub struct PaneGridElement {
    contents: ModelHandle<HerdrPaneContents>,
    style: PaneGridStyle,
    input: PaneInputTarget,
    size: Option<Vector2F>,
    origin: Option<Point>,
    /// The size of one cell as drawn, set once there is a screen to measure it against.
    drawn_cell: Option<Vector2F>,
}

impl PaneGridElement {
    pub fn new(
        contents: ModelHandle<HerdrPaneContents>,
        style: PaneGridStyle,
        input: PaneInputTarget,
    ) -> Self {
        Self {
            contents,
            style,
            input,
            size: None,
            origin: None,
            drawn_cell: None,
        }
    }

    fn receives_input(&self, app: &AppContext) -> bool {
        self.input.selected
            && app.check_view_or_child_focused(self.input.window_id, &self.input.view_id)
    }

    fn forward(&self, input: PaneInput, app: &AppContext) {
        self.contents.as_ref(app).stream().forward(input);
    }

    /// The cell of the pane's screen under `position`, in window coordinates.
    fn cell_under(&self, position: Vector2F, app: &AppContext) -> Option<(u16, u16)> {
        let origin = self.origin?;
        let cell = self.drawn_cell?;
        let terminal = self.contents.as_ref(app).terminal()?;
        Some(cell_at(
            position - origin.xy(),
            cell,
            terminal.cols(),
            terminal.rows(),
        ))
    }

    fn mouse(&self, action: MouseAction, position: Vector2F, app: &AppContext) -> bool {
        let Some((column, row)) = self.cell_under(position, app) else {
            return false;
        };
        self.forward(
            PaneInput::Mouse {
                action,
                column,
                row,
            },
            app,
        );
        true
    }
}

/// The factor to draw a grid of `cols` by `rows` cells of `cell` pixels with so that it fits
/// `frame`: 1 when it already does.
pub(super) fn contested_scale(frame: Vector2F, cell: Vector2F, cols: u16, rows: u16) -> f32 {
    let grid = vec2f(f32::from(cols) * cell.x(), f32::from(rows) * cell.y());
    if grid.x() <= 0. || grid.y() <= 0. {
        return 1.;
    }
    (frame.x() / grid.x())
        .min(frame.y() / grid.y())
        .clamp(0., 1.)
}

impl PaneGridElement {
    fn paint_cursor(
        &self,
        cursor: PaneCursor,
        origin: Vector2F,
        cell: Vector2F,
        ctx: &mut PaintContext,
    ) {
        let cell_origin = origin + cell * vec2f(f32::from(cursor.col), f32::from(cursor.row));
        let thickness = (CURSOR_THICKNESS_SCALE_FACTOR * cell.x().round()).max(1.);
        let fill = self.style.theme.cursor();
        match cursor.shape {
            CursorShape::Block => {
                ctx.scene
                    .draw_rect_with_hit_recording(RectF::new(cell_origin, cell))
                    .with_background(fill);
            }
            CursorShape::Underline => {
                ctx.scene
                    .draw_rect_with_hit_recording(RectF::new(
                        cell_origin + vec2f(0., cell.y() - thickness),
                        vec2f(cell.x(), thickness),
                    ))
                    .with_background(fill);
            }
            CursorShape::Beam => {
                ctx.scene
                    .draw_rect_with_hit_recording(RectF::new(
                        cell_origin,
                        vec2f(thickness, cell.y()),
                    ))
                    .with_background(fill);
            }
            CursorShape::HollowBlock => {
                ctx.scene
                    .draw_rect_with_hit_recording(RectF::new(cell_origin, cell))
                    .with_border(Border::all(thickness).with_border_fill(fill));
            }
            CursorShape::Hidden => {}
        }
    }
}

impl Element for PaneGridElement {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        _ctx: &mut LayoutContext,
        _app: &AppContext,
    ) -> Vector2F {
        let size = vec2f(
            if constraint.max.x().is_finite() {
                constraint.max.x()
            } else {
                constraint.min.x()
            },
            if constraint.max.y().is_finite() {
                constraint.max.y()
            } else {
                constraint.min.y()
            },
        );
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, _ctx: &mut AfterLayoutContext, _app: &AppContext) {}

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        let size = self.size.unwrap_or_default();
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        let bounds = RectF::new(origin, size);
        ctx.scene
            .draw_rect_with_hit_recording(bounds)
            .with_background(self.style.theme.background());

        let cell = grid_cell_dimensions(
            app.font_cache(),
            self.style.font_family,
            self.style.font_size,
            self.style.line_height_ratio,
        );
        let contents = self.contents.as_ref(app);
        let contested = matches!(contents.ownership(), Some(PaneOwnership::Contested(_)));
        if !contested {
            let scale = ctx.scene.scale_factor();
            contents.stream().set_size(PaneSize::fit(
                size.x() * scale,
                size.y() * scale,
                cell.x() * scale,
                cell.y() * scale,
            ));
        }
        let Some(terminal) = contents.terminal() else {
            return;
        };

        let factor = if contested {
            contested_scale(size, cell, terminal.cols(), terminal.rows())
        } else {
            1.
        };
        self.drawn_cell = Some(cell * factor);
        let cursor = terminal.cursor();
        let grid = terminal.screen().grid_handler();
        let no_matches: Option<std::slice::Iter<'_, RangeInclusive<GridPoint>>> = None;
        ctx.scene.start_layer(ClipBounds::BoundedBy(bounds));
        render_grid(
            grid,
            0,
            usize::from(terminal.rows()),
            &color::List::from(&self.style.theme.clone().into()),
            &OverrideList::empty(),
            &self.style.theme,
            Properties::default(),
            self.style.font_family,
            self.style.font_size * factor,
            self.style.line_height_ratio,
            cell * factor,
            0.0.into_pixels(),
            origin,
            &mut CellGlyphCache::default(),
            255, /* alpha */
            None,
            None,
            no_matches,
            None,
            EnforceMinimumContrast::default(),
            ObfuscateSecrets::No,
            None,
            false, /* use_ligature_rendering */
            cursor.map(|cursor| cursor.shape),
            RespectDisplayedOutput::No,
            &HashMap::new(),
            None,
            false, /* hide_cursor_cell */
            ctx,
            app,
        );
        if let Some(cursor) = cursor {
            self.paint_cursor(cursor, origin, cell * factor, ctx);
        }
        ctx.scene.stop_layer();
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        let (Some(origin), Some(size)) = (self.origin, self.size) else {
            return false;
        };
        let Some(event) = event.at_z_index(origin.z_index(), ctx) else {
            return false;
        };
        let bounds = RectF::new(origin.xy(), size);
        let receives_input = self.receives_input(app);
        match event {
            Event::KeyDown {
                keystroke,
                is_composing: false,
                ..
            } if receives_input && is_paste(keystroke) => {
                ctx.dispatch_typed_action(HerdrWorkspaceViewAction::Paste(self.input.pane.clone()));
                true
            }
            Event::KeyDown {
                keystroke,
                is_composing: false,
                ..
            } if receives_input => match keystroke_input(keystroke) {
                Some(input) => {
                    self.forward(input, app);
                    true
                }
                None => false,
            },
            Event::TypedCharacters { chars } if receives_input && !chars.is_empty() => {
                self.forward(PaneInput::Text(chars.clone()), app);
                true
            }
            Event::ScrollWheel {
                position,
                delta,
                precise,
                ..
            } if receives_input && bounds.contains_point(*position) => {
                let cell_height = self.drawn_cell.map_or(0., |cell| cell.y());
                let (Some((direction, lines)), Some((column, row))) = (
                    scroll_lines(delta.y(), *precise, cell_height),
                    self.cell_under(*position, app),
                ) else {
                    return false;
                };
                self.forward(
                    PaneInput::Scroll {
                        direction,
                        lines,
                        column,
                        row,
                    },
                    app,
                );
                true
            }
            Event::LeftMouseDown { position, .. } if bounds.contains_point(*position) => {
                if receives_input && self.mouse(MouseAction::Down, *position, app) {
                    self.contents.as_ref(app).mouse_forwarded().set(true);
                } else {
                    ctx.dispatch_typed_action(HerdrWorkspaceViewAction::SelectPane(
                        self.input.pane.clone(),
                    ));
                }
                true
            }
            Event::LeftMouseDragged { position, .. }
                if self.contents.as_ref(app).mouse_forwarded().get() =>
            {
                self.mouse(MouseAction::Drag, *position, app)
            }
            Event::LeftMouseUp { position, .. }
                if self.contents.as_ref(app).mouse_forwarded().replace(false) =>
            {
                self.mouse(MouseAction::Up, *position, app)
            }
            _ => false,
        }
    }
}
