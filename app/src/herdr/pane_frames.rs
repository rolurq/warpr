use herdr_client::CellRect;
use warpui::elements::Point;
use warpui::event::DispatchedEvent;
use warpui::geometry::rect::RectF;
use warpui::geometry::vector::{Vector2F, vec2f};
use warpui::{
    AfterLayoutContext, AppContext, Element, EventContext, LayoutContext, PaintContext,
    SizeConstraint,
};

/// Lays children out at the cell rects Herdr gave their panes, scaled to fill the space this
/// element is given.
pub struct PaneFrames {
    area: CellRect,
    frames: Vec<(CellRect, Box<dyn Element>)>,
    size: Option<Vector2F>,
    origin: Option<Point>,
}

impl PaneFrames {
    /// `area` is the space the frames' rects are measured in.
    pub fn new(area: CellRect, frames: Vec<(CellRect, Box<dyn Element>)>) -> Self {
        Self {
            area,
            frames,
            size: None,
            origin: None,
        }
    }
}

/// Where the cells `rect` falls within `area` when `area` is drawn at `size` pixels, as an offset
/// from the area's origin. Each edge is rounded on its own, so panes that share an edge in Herdr
/// share it on screen too, with no gap or overlap.
pub(super) fn placement(area: CellRect, rect: CellRect, size: Vector2F) -> RectF {
    if area.width == 0 || area.height == 0 {
        return RectF::new(Vector2F::zero(), Vector2F::zero());
    }
    let edge = |cells: u16, cells_in_area: u16, pixels: f32| {
        (f32::from(cells) / f32::from(cells_in_area) * pixels).round()
    };
    let left = edge(rect.x.saturating_sub(area.x), area.width, size.x());
    let top = edge(rect.y.saturating_sub(area.y), area.height, size.y());
    let right = edge(
        rect.x.saturating_add(rect.width).saturating_sub(area.x),
        area.width,
        size.x(),
    );
    let bottom = edge(
        rect.y.saturating_add(rect.height).saturating_sub(area.y),
        area.height,
        size.y(),
    );
    RectF::from_points(vec2f(left, top), vec2f(right, bottom))
}

impl Element for PaneFrames {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let size = vec2f(
            finite_or(constraint.max.x(), constraint.min.x()),
            finite_or(constraint.max.y(), constraint.min.y()),
        );
        for (rect, child) in &mut self.frames {
            let frame = placement(self.area, *rect, size);
            child.layout(SizeConstraint::strict(frame.size()), ctx, app);
        }
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        for (_, child) in &mut self.frames {
            child.after_layout(ctx, app);
        }
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        let size = self.size.unwrap_or_default();
        for (rect, child) in &mut self.frames {
            let frame = placement(self.area, *rect, size);
            child.paint(origin + frame.origin(), ctx, app);
        }
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
        self.frames.iter_mut().fold(false, |handled, (_, child)| {
            child.dispatch_event(event, ctx, app) || handled
        })
    }

    #[cfg(test)]
    fn debug_text_content(&self) -> Option<String> {
        let texts: Vec<_> = self
            .frames
            .iter()
            .filter_map(|(_, child)| child.debug_text_content())
            .collect();
        (!texts.is_empty()).then(|| texts.join(" "))
    }
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

#[cfg(test)]
#[path = "pane_frames_tests.rs"]
mod tests;
