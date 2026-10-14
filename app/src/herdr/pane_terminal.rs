use std::io;

use async_channel::Receiver;
use herdr_client::PaneFrame;

use crate::terminal::SizeInfo;
use crate::terminal::event::Event as TerminalEvent;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::alt_screen::AltScreen;
use crate::terminal::model::ansi::Processor;
use crate::terminal::model::grid::Dimensions as _;
use crate::terminal::model::secrets::ObfuscateSecrets;

/// A pane's screen, kept by feeding the ANSI bytes of Herdr's frames to Warp's VT parser.
///
/// A frame is a repaint of the whole visible screen, so the grid is an alternate screen with no
/// scrollback; it carries none of the block list and shell bootstrap a full terminal model would.
pub struct PaneTerminal {
    screen: AltScreen,
    parser: Processor,
    /// What the grid reports about itself (title changes, bells). Nothing here shows them, but a
    /// closed channel would log a warning for each.
    events: Receiver<TerminalEvent>,
}

impl PaneTerminal {
    pub fn new(cols: u16, rows: u16) -> Self {
        let (wakeups_tx, _) = async_channel::unbounded();
        let (events_tx, events) = async_channel::unbounded();
        let (pty_reads_tx, _) = async_broadcast::broadcast(1);
        let listener = ChannelEventListener::new(wakeups_tx, events_tx, pty_reads_tx);
        let screen = AltScreen::new(
            SizeInfo::new_without_font_metrics(usize::from(rows), usize::from(cols)),
            0, /* max_scroll_limit */
            listener,
            ObfuscateSecrets::No,
        );
        Self {
            screen,
            parser: Processor::default(),
            events,
        }
    }

    /// Shows `frame`. A frame drawn for another grid than the current one replaces the screen,
    /// because Herdr follows every size change with a full frame.
    pub fn apply(&mut self, frame: &PaneFrame) {
        if (frame.width, frame.height) != (self.cols(), self.rows()) {
            *self = Self::new(frame.width, frame.height);
        }
        self.parser
            .parse_bytes(&mut self.screen, &frame.bytes, &mut io::sink());
        while self.events.try_recv().is_ok() {}
    }

    pub fn cols(&self) -> u16 {
        self.screen.grid_handler().columns() as u16
    }

    pub fn rows(&self) -> u16 {
        self.screen.grid_handler().visible_rows() as u16
    }

    /// The screen's text, one line per row.
    #[cfg(test)]
    pub fn text(&self) -> String {
        self.screen.output_to_string()
    }

    pub(super) fn screen(&self) -> &AltScreen {
        &self.screen
    }
}

#[cfg(test)]
#[path = "pane_terminal_tests.rs"]
mod tests;
