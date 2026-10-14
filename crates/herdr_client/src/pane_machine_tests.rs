use super::*;

const INTERVAL: Duration = Duration::from_millis(100);
const GRACE: Duration = Duration::from_secs(10);

fn size(cols: u16, rows: u16) -> PaneSize {
    PaneSize {
        cols,
        rows,
        cell_width_px: 18,
        cell_height_px: 36,
    }
}

fn grid(cols: u16, rows: u16) -> GridSize {
    GridSize { cols, rows }
}

/// A machine plus the clock the tests move.
struct Clock {
    machine: Machine,
    now: Instant,
}

impl Clock {
    fn new() -> Self {
        Self {
            machine: Machine::new(Timing {
                resize_interval: INTERVAL,
                release_grace: GRACE,
            }),
            now: Instant::now(),
        }
    }

    fn input(&mut self, input: Input) -> Vec<Action> {
        self.machine.handle(self.now, input)
    }

    fn after(&mut self, wait: Duration, input: Input) -> Vec<Action> {
        self.now += wait;
        self.input(input)
    }

    /// A visible pane whose control session is running at `size`.
    fn controlling(size: PaneSize) -> Self {
        let mut clock = Self::new();
        clock.input(Input::Size(size));
        clock.input(Input::Visible(true));
        clock.input(Input::ControlFrame);
        clock
    }
}

#[test]
fn control_starts_once_the_pane_is_visible_and_sized_and_sends_the_pixel_size() {
    let mut clock = Clock::new();

    assert_eq!(clock.input(Input::Size(size(80, 24))), []);
    assert_eq!(
        clock.input(Input::Visible(true)),
        [
            Action::SpawnControl(size(80, 24)),
            Action::SendResize(size(80, 24))
        ]
    );
}

#[test]
fn control_does_not_start_for_a_hidden_pane() {
    let mut clock = Clock::new();

    assert_eq!(clock.input(Input::Visible(false)), []);
    assert_eq!(clock.input(Input::Size(size(80, 24))), []);
}

#[test]
fn first_control_frame_makes_the_pane_owned_once() {
    let mut clock = Clock::new();
    clock.input(Input::Size(size(80, 24)));
    clock.input(Input::Visible(true));

    assert_eq!(
        clock.input(Input::ControlFrame),
        [Action::Publish(PaneOwnership::Owned)]
    );
    assert_eq!(clock.input(Input::ControlFrame), []);
}

#[test]
fn resizes_are_throttled_leading_and_trailing_and_skip_the_size_already_sent() {
    let mut clock = Clock::controlling(size(80, 24));

    // Within the interval of the send that started the session: held back.
    assert_eq!(
        clock.after(Duration::from_millis(10), Input::Size(size(81, 24))),
        []
    );
    assert_eq!(
        clock.after(Duration::from_millis(10), Input::Size(size(82, 24))),
        []
    );
    assert_eq!(
        clock.machine.next_deadline(),
        Some(clock.now - Duration::from_millis(20) + INTERVAL)
    );
    // The trailing send carries only the latest size.
    assert_eq!(
        clock.after(Duration::from_millis(80), Input::Tick),
        [Action::SendResize(size(82, 24))]
    );
    // Back to what was just sent: nothing to send, and nothing left pending.
    assert_eq!(
        clock.after(Duration::from_millis(10), Input::Size(size(83, 24))),
        []
    );
    assert_eq!(
        clock.after(Duration::from_millis(10), Input::Size(size(82, 24))),
        []
    );
    assert_eq!(clock.after(INTERVAL, Input::Tick), []);
    // After a quiet interval a change goes out at once.
    assert_eq!(
        clock.after(INTERVAL, Input::Size(size(90, 30))),
        [Action::SendResize(size(90, 30))]
    );
}

#[test]
fn a_changed_pixel_size_alone_is_sent() {
    let mut clock = Clock::controlling(size(80, 24));
    let sharper = PaneSize {
        cell_width_px: 20,
        ..size(80, 24)
    };

    assert_eq!(
        clock.after(INTERVAL, Input::Size(sharper)),
        [Action::SendResize(sharper)]
    );
}

#[test]
fn refused_control_falls_back_to_observing_at_herdrs_grid() {
    let mut clock = Clock::new();
    clock.input(Input::Grid(grid(60, 20)));
    clock.input(Input::Size(size(80, 24)));
    clock.input(Input::Visible(true));

    assert_eq!(
        clock.input(Input::ControlClosed(Closure::Refused)),
        [
            Action::Publish(PaneOwnership::Contested(Contest::Refused)),
            Action::SpawnObserve(grid(60, 20)),
        ]
    );
}

#[test]
fn eviction_falls_back_to_observing_and_never_retries_control() {
    let mut clock = Clock::controlling(size(80, 24));
    clock.input(Input::Grid(grid(60, 20)));

    assert_eq!(
        clock.after(
            Duration::from_secs(1),
            Input::ControlClosed(Closure::TakenOver)
        ),
        [
            Action::Publish(PaneOwnership::Contested(Contest::Evicted)),
            Action::SpawnObserve(grid(60, 20)),
        ]
    );
    assert_eq!(
        clock.after(Duration::from_secs(1), Input::Size(size(90, 30))),
        []
    );
    assert_eq!(clock.after(Duration::from_secs(5), Input::Tick), []);
}

#[test]
fn a_contested_pane_restarts_observing_when_herdrs_grid_changes_throttled() {
    let mut clock = Clock::new();
    clock.input(Input::Grid(grid(60, 20)));
    clock.input(Input::Size(size(80, 24)));
    clock.input(Input::Visible(true));
    clock.input(Input::ControlClosed(Closure::Refused));

    assert_eq!(
        clock.after(Duration::from_millis(10), Input::Grid(grid(50, 20))),
        []
    );
    assert_eq!(
        clock.after(Duration::from_millis(100), Input::Tick),
        [Action::ReplaceObserve(grid(50, 20))]
    );
}

#[test]
fn an_exited_pane_is_reported_and_not_restarted() {
    let mut clock = Clock::controlling(size(80, 24));

    assert_eq!(
        clock.input(Input::ControlClosed(Closure::Exited)),
        [Action::Publish(PaneOwnership::Exited)]
    );
    clock.input(Input::Visible(false));
    assert_eq!(clock.after(GRACE, Input::Tick), []);
    clock.input(Input::Visible(true));
    assert_eq!(clock.after(INTERVAL, Input::Tick), []);
}

#[test]
fn a_failed_stream_is_not_restarted_until_the_pane_is_hidden_and_shown() {
    let mut clock = Clock::controlling(size(80, 24));

    assert_eq!(
        clock.input(Input::ControlClosed(Closure::Other(
            "server is shutting down".into()
        ))),
        [Action::Publish(PaneOwnership::Unavailable(
            "server is shutting down".into()
        ))]
    );
    assert_eq!(clock.after(Duration::from_secs(1), Input::Tick), []);
    clock.input(Input::Visible(false));
    assert_eq!(
        clock.after(Duration::from_secs(1), Input::Visible(true)),
        [
            Action::SpawnControl(size(80, 24)),
            Action::SendResize(size(80, 24))
        ]
    );
}

/// A visible pane whose control was refused, so it is being watched at `grid`.
fn contested(size: PaneSize, grid: GridSize) -> Clock {
    let mut clock = Clock::new();
    clock.input(Input::Grid(grid));
    clock.input(Input::Size(size));
    clock.input(Input::Visible(true));
    clock.input(Input::ControlClosed(Closure::Refused));
    clock
}

#[test]
fn forwarded_input_takes_a_contested_pane_at_the_pane_size_and_reports_it() {
    let mut clock = contested(size(80, 24), grid(60, 20));

    assert_eq!(
        clock.input(Input::Forward),
        [
            Action::StopObserve,
            Action::TakeOverControl(size(80, 24)),
            Action::SendResize(size(80, 24)),
        ]
    );
    assert_eq!(
        clock.input(Input::ControlFrame),
        [Action::Publish(PaneOwnership::TookControl)]
    );
    assert_eq!(clock.input(Input::Forward), []);
}

#[test]
fn a_pane_acquired_again_after_a_takeover_is_plainly_owned() {
    let mut clock = contested(size(80, 24), grid(60, 20));
    clock.input(Input::Forward);
    clock.input(Input::ControlFrame);
    clock.input(Input::Visible(false));
    clock.after(GRACE, Input::Tick);
    clock.input(Input::Visible(true));

    assert_eq!(
        clock.input(Input::ControlFrame),
        [Action::Publish(PaneOwnership::Owned)]
    );
}

#[test]
fn forwarded_input_changes_nothing_unless_the_pane_is_contested() {
    let mut idle = Clock::new();
    assert_eq!(idle.input(Input::Forward), []);

    let mut owned = Clock::controlling(size(80, 24));
    assert_eq!(owned.input(Input::Forward), []);

    let mut over = Clock::controlling(size(80, 24));
    over.input(Input::ControlClosed(Closure::Exited));
    assert_eq!(over.input(Input::Forward), []);
}

#[test]
fn control_is_released_only_after_the_grace_and_acquired_again_on_return() {
    let mut clock = Clock::controlling(size(80, 24));

    assert_eq!(clock.input(Input::Visible(false)), []);
    assert_eq!(clock.machine.next_deadline(), Some(clock.now + GRACE));
    assert_eq!(
        clock.after(GRACE - Duration::from_millis(1), Input::Tick),
        []
    );
    assert_eq!(
        clock.after(Duration::from_millis(1), Input::Tick),
        [
            Action::ReleaseControl,
            Action::Publish(PaneOwnership::Released)
        ]
    );
    assert_eq!(clock.machine.next_deadline(), None);
    assert_eq!(
        clock.after(Duration::from_secs(30), Input::Visible(true)),
        [
            Action::SpawnControl(size(80, 24)),
            Action::SendResize(size(80, 24))
        ]
    );
}

#[test]
fn flicking_between_herdr_tabs_within_the_grace_never_touches_ownership() {
    let mut clock = Clock::controlling(size(80, 24));

    for _ in 0..5 {
        assert_eq!(
            clock.after(Duration::from_secs(1), Input::Visible(false)),
            []
        );
        assert_eq!(
            clock.after(Duration::from_secs(8), Input::Visible(true)),
            []
        );
    }
    assert_eq!(clock.after(GRACE, Input::Tick), []);
}

#[test]
fn a_contested_pane_that_stays_hidden_is_released_and_then_tries_control_again() {
    let mut clock = Clock::new();
    clock.input(Input::Grid(grid(60, 20)));
    clock.input(Input::Size(size(80, 24)));
    clock.input(Input::Visible(true));
    clock.input(Input::ControlClosed(Closure::Refused));
    clock.input(Input::Visible(false));

    assert_eq!(
        clock.after(GRACE, Input::Tick),
        [
            Action::StopObserve,
            Action::Publish(PaneOwnership::Released)
        ]
    );
    assert_eq!(
        clock.after(Duration::from_secs(1), Input::Visible(true)),
        [
            Action::SpawnControl(size(80, 24)),
            Action::SendResize(size(80, 24))
        ]
    );
}

#[test]
fn closures_are_classified_from_herdrs_reasons() {
    let reasons = [
        (
            "terminal attach failed: terminal term_ab already has an attached client; retry with --takeover",
            Closure::Refused,
        ),
        ("terminal attach taken over", Closure::TakenOver),
        ("terminal term_ab exited", Closure::Exited),
        (
            "server is shutting down",
            Closure::Other("server is shutting down".into()),
        ),
    ];
    for (reason, expected) in reasons {
        assert_eq!(Closure::from_reason(Some(reason)), expected, "{reason}");
    }
}
