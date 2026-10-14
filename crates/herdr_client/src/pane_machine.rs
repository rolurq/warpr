use std::time::Duration;

use instant::Instant;

use crate::pane_size::{GridSize, PaneSize, Throttle};

/// How a pane's stream relates to the pane: whether warpr controls it, only watches it, or has
/// nothing to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneOwnership {
    /// warpr holds the pane's control session, so the pane has the size warpr chose.
    Owned,
    /// Like [`Self::Owned`], after input forwarded to a contested pane took it from another
    /// client.
    TookControl,
    /// Another client owns the pane; warpr watches it at Herdr's own grid size.
    Contested(Contest),
    /// The pane's process exited.
    Exited,
    /// The pane is not visible and warpr let go of it.
    Released,
    /// The stream ended for a reason that is neither contest nor exit.
    Unavailable(String),
}

/// How warpr learned that another client owns the pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Contest {
    /// Control was refused because the pane already had an owner.
    Refused,
    /// The owner took the pane from warpr.
    Evicted,
}

/// Why a stream ended, read from its `terminal.closed` record. Herdr's reasons are prose, not
/// codes, so they are matched on their wording.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Closure {
    Refused,
    TakenOver,
    Exited,
    Other(String),
}

impl Closure {
    /// `reason` is `None` when the process ended without a `terminal.closed` record, which only a
    /// failed connection does.
    pub(crate) fn from_reason(reason: Option<&str>) -> Self {
        match reason {
            Some(reason) if reason.contains("already has an attached client") => Self::Refused,
            Some(reason) if reason.contains("taken over") => Self::TakenOver,
            Some(reason) if reason.contains("exited") => Self::Exited,
            Some(reason) => Self::Other(reason.to_owned()),
            None => Self::Other("herdr ended the stream without saying why".to_owned()),
        }
    }
}

pub(crate) enum Input {
    Visible(bool),
    Size(PaneSize),
    Grid(GridSize),
    /// Input is being forwarded to the pane, which a contested pane answers by taking control.
    Forward,
    ControlFrame,
    ControlClosed(Closure),
    ObserveClosed(Closure),
    Tick,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Action {
    SpawnControl(PaneSize),
    /// Starts a control session that evicts the pane's current owner.
    TakeOverControl(PaneSize),
    SendResize(PaneSize),
    /// Gives up control by asking Herdr to release the pane and closing the session.
    ReleaseControl,
    SpawnObserve(GridSize),
    /// Starts an observe stream at the new grid beside the running one, which is stopped once the
    /// new one has produced a frame: an observe stream cannot be resized in place.
    ReplaceObserve(GridSize),
    StopObserve,
    Publish(PaneOwnership),
}

#[derive(Clone, Copy)]
pub(crate) struct Timing {
    pub(crate) resize_interval: Duration,
    pub(crate) release_grace: Duration,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Link {
    /// Nothing runs; control is tried as soon as the pane is visible.
    Idle,
    Control,
    Observe(GridSize),
    /// The stream ended badly. It is tried again only after the pane was hidden and shown.
    Failed,
    /// The pane's process exited.
    Over,
}

/// Decides what a pane's stream does next. It performs nothing and reads no clock, so every rule
/// of the resize and ownership policy can be tested with made-up times.
pub(crate) struct Machine {
    timing: Timing,
    visible: bool,
    hidden_since: Option<Instant>,
    size: Option<PaneSize>,
    grid: Option<GridSize>,
    link: Link,
    owned: bool,
    /// Whether the running control session was started by taking the pane from another client.
    took_over: bool,
    resize: Throttle<PaneSize>,
    observe_grid: Throttle<GridSize>,
}

impl Machine {
    pub(crate) fn new(timing: Timing) -> Self {
        Self {
            timing,
            visible: false,
            hidden_since: None,
            size: None,
            grid: None,
            link: Link::Idle,
            owned: false,
            took_over: false,
            resize: Throttle::new(timing.resize_interval),
            observe_grid: Throttle::new(timing.resize_interval),
        }
    }

    pub(crate) fn handle(&mut self, now: Instant, input: Input) -> Vec<Action> {
        let mut actions = Vec::new();
        match input {
            Input::Visible(visible) => self.set_visible(now, visible),
            Input::Size(size) => {
                self.size = Some(size);
                if self.link == Link::Control {
                    actions.extend(self.resize.offer(now, size).map(Action::SendResize));
                }
            }
            Input::Grid(grid) => {
                self.grid = Some(grid);
                if matches!(self.link, Link::Observe(_))
                    && let Some(grid) = self.observe_grid.offer(now, grid)
                {
                    self.link = Link::Observe(grid);
                    actions.push(Action::ReplaceObserve(grid));
                }
            }
            Input::ControlFrame => {
                if self.link == Link::Control && !self.owned {
                    self.owned = true;
                    actions.push(Action::Publish(if self.took_over {
                        PaneOwnership::TookControl
                    } else {
                        PaneOwnership::Owned
                    }));
                }
            }
            Input::Forward => self.take_over(now, &mut actions),
            Input::ControlClosed(closure) => {
                if self.link == Link::Control {
                    self.control_ended(now, closure, &mut actions);
                }
            }
            Input::ObserveClosed(closure) => {
                if matches!(self.link, Link::Observe(_)) {
                    self.observe_ended(closure, &mut actions);
                }
            }
            Input::Tick => {}
        }
        self.advance(now, &mut actions);
        actions
    }

    /// When [`Input::Tick`] next has something to do.
    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        let release = match self.link {
            Link::Control | Link::Observe(_) => self
                .hidden_since
                .map(|since| since + self.timing.release_grace),
            Link::Idle | Link::Failed | Link::Over => None,
        };
        let throttle = match self.link {
            Link::Control => self.resize.deadline(),
            Link::Observe(_) => self.observe_grid.deadline(),
            Link::Idle | Link::Failed | Link::Over => None,
        };
        [release, throttle].into_iter().flatten().min()
    }

    fn set_visible(&mut self, now: Instant, visible: bool) {
        if visible == self.visible {
            return;
        }
        self.visible = visible;
        if visible {
            self.hidden_since = None;
            if self.link == Link::Failed {
                self.link = Link::Idle;
            }
        } else {
            self.hidden_since = Some(now);
        }
    }

    fn control_ended(&mut self, now: Instant, closure: Closure, actions: &mut Vec<Action>) {
        match closure {
            Closure::Refused => self.contest(now, Contest::Refused, actions),
            Closure::TakenOver => self.contest(now, Contest::Evicted, actions),
            Closure::Exited => self.exited(actions),
            Closure::Other(reason) => self.failed(reason, actions),
        }
    }

    fn observe_ended(&mut self, closure: Closure, actions: &mut Vec<Action>) {
        actions.push(Action::StopObserve);
        match closure {
            Closure::Exited => self.exited(actions),
            Closure::Refused | Closure::TakenOver => {
                self.failed(
                    "an observe stream was closed as an owner's".to_owned(),
                    actions,
                );
            }
            Closure::Other(reason) => self.failed(reason, actions),
        }
    }

    /// Takes the pane from its owner. Nothing happens unless warpr is watching the pane and knows
    /// the size to hold it at.
    fn take_over(&mut self, now: Instant, actions: &mut Vec<Action>) {
        let (Link::Observe(_), Some(size)) = (self.link, self.size) else {
            return;
        };
        self.link = Link::Control;
        self.took_over = true;
        self.owned = false;
        self.resize = Throttle::new(self.timing.resize_interval);
        actions.push(Action::StopObserve);
        actions.push(Action::TakeOverControl(size));
        actions.extend(self.resize.offer(now, size).map(Action::SendResize));
    }

    fn contest(&mut self, now: Instant, contest: Contest, actions: &mut Vec<Action>) {
        // Control only ever ends this way on a pane that has a size, and a grid is only missing
        // when the caller never gave one, in which case the control size is the best there is.
        let grid = self
            .grid
            .or_else(|| self.size.map(PaneSize::grid))
            .expect("control was started with a size");
        self.link = Link::Observe(grid);
        self.owned = false;
        self.took_over = false;
        self.observe_grid = Throttle::new(self.timing.resize_interval);
        self.observe_grid.offer(now, grid);
        actions.push(Action::Publish(PaneOwnership::Contested(contest)));
        actions.push(Action::SpawnObserve(grid));
    }

    fn exited(&mut self, actions: &mut Vec<Action>) {
        self.link = Link::Over;
        self.owned = false;
        self.took_over = false;
        actions.push(Action::Publish(PaneOwnership::Exited));
    }

    fn failed(&mut self, reason: String, actions: &mut Vec<Action>) {
        self.link = Link::Failed;
        self.owned = false;
        self.took_over = false;
        actions.push(Action::Publish(PaneOwnership::Unavailable(reason)));
    }

    /// Applies everything that depends on time or on the state the input left behind.
    fn advance(&mut self, now: Instant, actions: &mut Vec<Action>) {
        match self.link {
            Link::Control => actions.extend(self.resize.poll(now).map(Action::SendResize)),
            Link::Observe(_) => {
                if let Some(grid) = self.observe_grid.poll(now) {
                    self.link = Link::Observe(grid);
                    actions.push(Action::ReplaceObserve(grid));
                }
            }
            Link::Idle | Link::Failed | Link::Over => {}
        }

        let grace_over = self
            .hidden_since
            .is_some_and(|since| now >= since + self.timing.release_grace);
        match self.link {
            Link::Control if grace_over => {
                actions.push(Action::ReleaseControl);
                self.release(actions);
            }
            Link::Observe(_) if grace_over => {
                actions.push(Action::StopObserve);
                self.release(actions);
            }
            Link::Control | Link::Observe(_) | Link::Idle | Link::Failed | Link::Over => {}
        }

        if self.link == Link::Idle
            && self.visible
            && let Some(size) = self.size
        {
            self.link = Link::Control;
            self.resize = Throttle::new(self.timing.resize_interval);
            actions.push(Action::SpawnControl(size));
            // Acquiring zeroes the pixel size, so the size goes out even though it is the one
            // the session was started with.
            actions.extend(self.resize.offer(now, size).map(Action::SendResize));
        }
    }

    fn release(&mut self, actions: &mut Vec<Action>) {
        self.link = Link::Idle;
        self.owned = false;
        self.took_over = false;
        self.hidden_since = None;
        actions.push(Action::Publish(PaneOwnership::Released));
    }
}

#[cfg(test)]
#[path = "pane_machine_tests.rs"]
mod tests;
