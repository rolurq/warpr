use std::time::Duration;

use instant::Instant;

/// The size warpr holds a pane's control session at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneSize {
    pub cols: u16,
    pub rows: u16,
    /// The size of one terminal cell in physical pixels, as the program in the pane reads it.
    pub cell_width_px: u32,
    pub cell_height_px: u32,
}

impl PaneSize {
    /// The whole cells that fit a frame of `frame_width_px` by `frame_height_px` physical pixels.
    ///
    /// Cells are floor-snapped and never fewer than one, because Herdr rejects a zero size; the
    /// remainder is the caller's padding. A cell's pixel size is rounded to whole pixels first, so
    /// the reported size and the fitted columns agree.
    pub fn fit(
        frame_width_px: f32,
        frame_height_px: f32,
        cell_width_px: f32,
        cell_height_px: f32,
    ) -> Self {
        let cell_width_px = whole_pixels(cell_width_px);
        let cell_height_px = whole_pixels(cell_height_px);
        Self {
            cols: whole_cells(frame_width_px, cell_width_px),
            rows: whole_cells(frame_height_px, cell_height_px),
            cell_width_px,
            cell_height_px,
        }
    }

    pub fn grid(self) -> GridSize {
        GridSize {
            cols: self.cols,
            rows: self.rows,
        }
    }
}

/// A grid of cells, without a pixel size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSize {
    pub cols: u16,
    pub rows: u16,
}

fn whole_pixels(pixels: f32) -> u32 {
    (pixels.round() as u32).max(1)
}

fn whole_cells(frame_px: f32, cell_px: u32) -> u16 {
    ((frame_px / cell_px as f32).floor() as u32).clamp(1, u32::from(u16::MAX)) as u16
}

/// A leading and trailing throttle over a stream of values: the first value goes out at once, later
/// ones at most once per interval, and the latest value always goes out after the last change.
/// A value equal to the last one sent is never sent again.
///
/// It reads no clock; callers pass the time, which keeps it testable without sleeping.
pub(crate) struct Throttle<T> {
    interval: Duration,
    last_sent: Option<(Instant, T)>,
    pending: Option<T>,
}

impl<T: Copy + PartialEq> Throttle<T> {
    pub(crate) fn new(interval: Duration) -> Self {
        Self {
            interval,
            last_sent: None,
            pending: None,
        }
    }

    /// Offers the newest value; returns it when it is to be sent now.
    pub(crate) fn offer(&mut self, now: Instant, value: T) -> Option<T> {
        if self.last_sent.is_some_and(|(_, sent)| sent == value) {
            self.pending = None;
            return None;
        }
        self.pending = Some(value);
        self.poll(now)
    }

    /// Returns the pending value when its interval has passed.
    pub(crate) fn poll(&mut self, now: Instant) -> Option<T> {
        let value = self.pending?;
        if self.deadline().is_some_and(|due| now < due) {
            return None;
        }
        self.pending = None;
        self.last_sent = Some((now, value));
        Some(value)
    }

    /// When a pending value becomes due.
    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.pending?;
        self.last_sent.map(|(sent_at, _)| sent_at + self.interval)
    }
}

#[cfg(test)]
#[path = "pane_size_tests.rs"]
mod tests;
