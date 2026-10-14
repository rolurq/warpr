use std::collections::HashMap;
use std::ops::RangeInclusive;

use herdr_client::{PaneOwnership, PaneSize};
use warp_core::ui::theme::WarpTheme;
use warpui::elements::Point;
use warpui::fonts::{FamilyId, Properties};
use warpui::geometry::rect::RectF;
use warpui::geometry::vector::{Vector2F, vec2f};
use warpui::units::IntoPixels;
use warpui::{
    AfterLayoutContext, AppContext, ClipBounds, Element, LayoutContext, ModelHandle, PaintContext,
    SizeConstraint,
};

use super::pane_contents::HerdrPaneContents;
use crate::settings::EnforceMinimumContrast;
use crate::terminal::color::{self, OverrideList};
use crate::terminal::grid_renderer::{CellGlyphCache, render_grid};
use crate::terminal::grid_size_util::grid_cell_dimensions;
use crate::terminal::model::grid::RespectDisplayedOutput;
use crate::terminal::model::index::Point as GridPoint;
use crate::terminal::model::secrets::ObfuscateSecrets;

/// How the cells of a pane's screen are drawn.
pub struct PaneGridStyle {
    pub theme: WarpTheme,
    pub font_family: FamilyId,
    pub font_size: f32,
    pub line_height_ratio: f32,
}

/// Paints the screen of a pane and tells its stream how many cells the frame holds.
///
/// A pane warpr controls is drawn at the font's own cell size and left padded where the frame is
/// not a whole number of cells. A contested pane is watched at Herdr's grid, which need not fit
/// the frame, so it is drawn at the font's size when it fits and shrunk with its font until it
/// does when it does not; what is left of the frame stays blank.
pub struct PaneGridElement {
    contents: ModelHandle<HerdrPaneContents>,
    style: PaneGridStyle,
    size: Option<Vector2F>,
    origin: Option<Point>,
}

impl PaneGridElement {
    pub fn new(contents: ModelHandle<HerdrPaneContents>, style: PaneGridStyle) -> Self {
        Self {
            contents,
            style,
            size: None,
            origin: None,
        }
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
            None,
            RespectDisplayedOutput::No,
            &HashMap::new(),
            None,
            true, /* hide_cursor_cell */
            ctx,
            app,
        );
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
        _event: &warpui::event::DispatchedEvent,
        _ctx: &mut warpui::EventContext,
        _app: &AppContext,
    ) -> bool {
        false
    }
}
