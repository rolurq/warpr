use std::fmt::Debug;
use std::sync::Arc;

use herdr_fake::{FakeFrame, FakeHerdr, Invocation, StreamKind, TerminalStreamScript};
use serde_json::json;

use super::*;
use crate::SessionName;

const WAIT_LIMIT: Duration = Duration::from_secs(10);
const PANE: &str = "w1:p1";
const REFUSED: &str = "terminal attach failed: terminal term_ab already has an attached client; retry with --takeover";
const EVICTED: &str = "terminal attach taken over";

/// Polls `observe` until what it returns satisfies `done`, and returns that.
fn eventually<T: Debug>(
    expectation: &str,
    mut observe: impl FnMut() -> T,
    done: impl Fn(&T) -> bool,
) -> T {
    let deadline = Instant::now() + WAIT_LIMIT;
    loop {
        let observed = observe();
        if done(&observed) {
            return observed;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {expectation}, last saw {observed:?}"
        );
        thread::sleep(Duration::from_millis(2));
    }
}

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

fn control() -> TerminalStreamScript {
    TerminalStreamScript::new()
}

fn target(fake: &FakeHerdr) -> SessionTarget {
    SessionTarget::new(
        &fake.config_dir(),
        SessionName::new("default").expect("valid name"),
    )
}

/// A pane stream together with every event it reported.
struct Watched {
    stream: PaneStream,
    events: Arc<Mutex<Vec<PaneStreamEvent>>>,
}

impl Watched {
    fn new(fake: &FakeHerdr, release_grace: Duration) -> Self {
        Self::on_pane(fake, PANE, release_grace)
    }

    fn on_pane(fake: &FakeHerdr, pane: &str, release_grace: Duration) -> Self {
        Self::with_cli(HerdrCli::new(fake.binary_path()), fake, pane, release_grace)
    }

    fn with_cli(cli: HerdrCli, fake: &FakeHerdr, pane: &str, release_grace: Duration) -> Self {
        let events = Arc::new(Mutex::new(Vec::new()));
        let stream = PaneStream::start_with_timing(
            cli,
            target(fake),
            PaneId::from(pane),
            Timing {
                resize_interval: Duration::from_millis(20),
                release_grace,
            },
            {
                let events = Arc::clone(&events);
                move |event| events.lock().push(event)
            },
        );
        Self { stream, events }
    }

    /// Shows the pane at `size`, with Herdr's own grid for it being `herdr_grid`.
    fn show(&self, size: PaneSize, herdr_grid: GridSize) {
        self.stream.set_herdr_grid(herdr_grid);
        self.stream.set_size(size);
        self.stream.set_visible(true);
    }

    fn events(&self) -> Vec<PaneStreamEvent> {
        self.events.lock().clone()
    }

    /// Waits until an event that is `wanted` has been reported, and returns every event so far.
    fn wait_for(
        &self,
        expectation: &str,
        wanted: impl Fn(&PaneStreamEvent) -> bool,
    ) -> Vec<PaneStreamEvent> {
        eventually(
            expectation,
            || self.events(),
            |events| events.iter().any(&wanted),
        )
    }

    fn wait_for_frame(&self, bytes: &[u8]) -> Vec<PaneStreamEvent> {
        self.wait_for(
            &format!("the frame {:?}", String::from_utf8_lossy(bytes)),
            |event| matches!(event, PaneStreamEvent::Frame(frame) if frame.bytes == bytes),
        )
    }

    fn wait_for_ownership(&self, ownership: PaneOwnership) -> Vec<PaneStreamEvent> {
        let wanted = PaneStreamEvent::Ownership(ownership);
        self.wait_for(&format!("{wanted:?}"), |event| *event == wanted)
    }

    /// Stops the stream once it has handled everything asked of it, and returns every event it
    /// reported, which can no longer change.
    fn stop(self) -> Vec<PaneStreamEvent> {
        drop(self.stream);
        self.events.lock().clone()
    }
}

fn frame(event: &PaneStreamEvent) -> &PaneFrame {
    match event {
        PaneStreamEvent::Frame(frame) => frame,
        PaneStreamEvent::Ownership(ownership) => panic!("expected a frame, got {ownership:?}"),
    }
}

fn runs_of(fake: &FakeHerdr, subcommand: &str) -> Vec<Invocation> {
    fake.invocations()
        .into_iter()
        .filter(|run| run.args.get(2).map(String::as_str) == Some(subcommand))
        .collect()
}

/// Waits until the control runs of the pane have received exactly `expected`.
fn wait_for_control_input(fake: &FakeHerdr, expected: &[serde_json::Value]) {
    eventually(
        "the control input",
        || fake.control_input(PANE),
        |input| input == expected,
    );
}

fn resize_line(size: PaneSize) -> serde_json::Value {
    json!({
        "type": "terminal.resize",
        "cols": size.cols,
        "rows": size.rows,
        "cell_width_px": size.cell_width_px,
        "cell_height_px": size.cell_height_px,
    })
}

#[test]
fn a_free_pane_is_controlled_at_the_chosen_size_and_its_frames_arrive_in_order() {
    let mut fake = FakeHerdr::new();
    fake.script_terminal_stream(
        StreamKind::Control,
        PANE,
        [control()
            .frame(FakeFrame::full(1, 80, 24, "first"))
            .frame(FakeFrame::incremental(2, 80, 24, "\x1b[2;1Hsecond"))
            .frame(FakeFrame::full(3, 80, 24, "\x1b[2Jthird"))],
    );
    let watched = Watched::new(&fake, RELEASE_GRACE);

    watched.show(size(80, 24), grid(78, 22));

    let events = watched.wait_for_frame(b"\x1b[2Jthird");
    assert_eq!(events[0], PaneStreamEvent::Ownership(PaneOwnership::Owned));
    let frames: Vec<_> = events[1..].iter().map(frame).collect();
    assert_eq!(
        frames
            .iter()
            .map(|frame| (frame.seq, frame.full, frame.bytes.as_slice()))
            .collect::<Vec<_>>(),
        [
            (1, true, b"first".as_slice()),
            (2, false, b"\x1b[2;1Hsecond".as_slice()),
            (3, true, b"\x1b[2Jthird".as_slice()),
        ]
    );
    assert_eq!(
        runs_of(&fake, "control")[0].args,
        [
            "terminal", "session", "control", PANE, "--cols", "80", "--rows", "24"
        ]
    );
    wait_for_control_input(&fake, &[resize_line(size(80, 24))]);
}

#[test]
fn streams_run_against_the_explicit_session_and_never_ask_for_takeover() {
    let mut fake = FakeHerdr::new();
    fake.script_terminal_stream(StreamKind::Control, PANE, [control().closed(REFUSED)]);
    fake.script_terminal_stream(
        StreamKind::Observe,
        PANE,
        [control().frame(FakeFrame::full(1, 60, 20, "x"))],
    );
    let watched = Watched::new(&fake, RELEASE_GRACE);
    watched.show(size(80, 24), grid(60, 20));
    watched.wait_for_frame(b"x");

    let runs = fake.invocations();
    assert!(!runs.is_empty());
    for run in &runs {
        assert!(!run.args.iter().any(|arg| arg == "--takeover"), "{run:?}");
        assert_eq!(
            run.env("HERDR_SOCKET_PATH"),
            Some(target(&fake).socket_path().to_str().unwrap())
        );
        assert_eq!(run.env("HERDR_SESSION"), Some("default"));
    }
}

#[test]
fn resizes_reach_the_pane_in_whole_cells_with_the_pixel_size_and_never_repeat() {
    let mut fake = FakeHerdr::new();
    fake.script_terminal_stream(
        StreamKind::Control,
        PANE,
        [control().frame(FakeFrame::full(1, 80, 24, "x"))],
    );
    let watched = Watched::new(&fake, RELEASE_GRACE);
    watched.show(size(80, 24), grid(78, 22));
    watched.wait_for_frame(b"x");

    let wider = size(100, 30);
    watched.stream.set_size(wider);
    wait_for_control_input(&fake, &[resize_line(size(80, 24)), resize_line(wider)]);
    watched.stream.set_size(wider);
    let taller = PaneSize {
        cell_height_px: 40,
        ..wider
    };
    watched.stream.set_size(taller);

    wait_for_control_input(
        &fake,
        &[
            resize_line(size(80, 24)),
            resize_line(wider),
            resize_line(taller),
        ],
    );
}

#[test]
fn refused_control_shows_the_pane_through_observe_at_herdrs_grid() {
    let mut fake = FakeHerdr::new();
    fake.script_terminal_stream(StreamKind::Control, PANE, [control().closed(REFUSED)]);
    fake.script_terminal_stream(
        StreamKind::Observe,
        PANE,
        [control().frame(FakeFrame::full(1, 60, 20, "watched"))],
    );
    let watched = Watched::new(&fake, RELEASE_GRACE);

    watched.show(size(80, 24), grid(60, 20));

    let events = watched.wait_for_frame(b"watched");
    assert_eq!(
        events[0],
        PaneStreamEvent::Ownership(PaneOwnership::Contested(Contest::Refused))
    );
    assert_eq!(frame(&events[1]).bytes, b"watched");
    assert_eq!(
        runs_of(&fake, "observe")[0].args,
        [
            "terminal", "session", "observe", PANE, "--cols", "60", "--rows", "20"
        ]
    );
    assert_eq!(runs_of(&fake, "control").len(), 1);
}

#[test]
fn eviction_mid_stream_lands_in_contested_observe_without_control_being_retried() {
    let mut fake = FakeHerdr::new();
    fake.script_terminal_stream(
        StreamKind::Control,
        PANE,
        [control()
            .frame(FakeFrame::full(1, 80, 24, "mine"))
            .wait_for_gate("evict")
            .closed(EVICTED)],
    );
    fake.script_terminal_stream(
        StreamKind::Observe,
        PANE,
        [control().frame(FakeFrame::full(1, 60, 20, "theirs"))],
    );
    let watched = Watched::new(&fake, RELEASE_GRACE);
    watched.show(size(80, 24), grid(60, 20));
    watched.wait_for_frame(b"mine");

    fake.open_gate("evict");

    let events = watched.wait_for_frame(b"theirs");
    assert_eq!(
        events[2],
        PaneStreamEvent::Ownership(PaneOwnership::Contested(Contest::Evicted))
    );
    assert_eq!(frame(&events[3]).bytes, b"theirs");
    watched.stream.set_size(size(120, 40));
    watched.stop();
    assert_eq!(runs_of(&fake, "control").len(), 1);
    assert!(
        fake.invocations()
            .iter()
            .all(|run| !run.args.iter().any(|arg| arg == "--takeover"))
    );
}

#[test]
fn a_pane_whose_process_exited_reports_it_and_is_not_restarted() {
    let mut fake = FakeHerdr::new();
    fake.script_terminal_stream(
        StreamKind::Control,
        PANE,
        [control()
            .frame(FakeFrame::full(1, 80, 24, "bye"))
            .closed("terminal term_ab exited")],
    );
    let watched = Watched::new(&fake, RELEASE_GRACE);

    watched.show(size(80, 24), grid(78, 22));

    let events = watched.wait_for_ownership(PaneOwnership::Exited);
    assert_eq!(events[2], PaneStreamEvent::Ownership(PaneOwnership::Exited));
    assert_eq!(watched.stop().len(), 3);
    assert_eq!(fake.invocations().len(), 1);
}

#[test]
fn a_pane_that_stays_hidden_is_released_after_the_grace_and_reacquired_when_shown() {
    let mut fake = FakeHerdr::new();
    fake.script_terminal_stream(
        StreamKind::Control,
        PANE,
        [
            control().frame(FakeFrame::full(1, 80, 24, "one")),
            control().frame(FakeFrame::full(1, 80, 24, "two")),
        ],
    );
    let watched = Watched::new(&fake, Duration::from_millis(150));
    watched.show(size(80, 24), grid(78, 22));
    watched.wait_for_frame(b"one");

    watched.stream.set_visible(false);
    let events = watched.wait_for_ownership(PaneOwnership::Released);
    assert_eq!(
        events[2],
        PaneStreamEvent::Ownership(PaneOwnership::Released)
    );
    wait_for_control_input(
        &fake,
        &[
            resize_line(size(80, 24)),
            json!({ "type": "terminal.release" }),
        ],
    );

    watched.stream.set_visible(true);
    let events = watched.wait_for_frame(b"two");
    assert_eq!(events[3], PaneStreamEvent::Ownership(PaneOwnership::Owned));
    assert_eq!(frame(&events[4]).bytes, b"two");
    assert_eq!(runs_of(&fake, "control").len(), 2);
}

#[test]
fn a_pane_shown_again_within_the_grace_keeps_its_control_session() {
    const STAYS_HIDDEN: &str = "w1:p2";
    let mut fake = FakeHerdr::new();
    for pane in [PANE, STAYS_HIDDEN] {
        fake.script_terminal_stream(
            StreamKind::Control,
            pane,
            [control().frame(FakeFrame::full(1, 80, 24, "one"))],
        );
    }
    let grace = Duration::from_millis(400);
    let watched = Watched::new(&fake, grace);
    // Hidden after the watched pane was hidden and shown again, so its release shows that the
    // watched pane's grace has run out too.
    let reference = Watched::on_pane(&fake, STAYS_HIDDEN, grace);
    for pane in [&watched, &reference] {
        pane.show(size(80, 24), grid(78, 22));
        pane.wait_for_frame(b"one");
    }

    watched.stream.set_visible(false);
    watched.stream.set_visible(true);
    let before = json!({ "type": "terminal.input", "text": "before" });
    watched.stream.forward(PaneInput::Text("before".to_owned()));
    wait_for_control_input(&fake, &[resize_line(size(80, 24)), before.clone()]);
    reference.stream.set_visible(false);
    reference.wait_for_ownership(PaneOwnership::Released);
    watched.stream.forward(PaneInput::Text("after".to_owned()));

    wait_for_control_input(
        &fake,
        &[
            resize_line(size(80, 24)),
            before,
            json!({ "type": "terminal.input", "text": "after" }),
        ],
    );
    assert_eq!(watched.stop().len(), 2);
    assert_eq!(
        runs_of(&fake, "control")
            .iter()
            .filter(|run| run.args[3] == PANE)
            .count(),
        1
    );
}

#[test]
fn a_changed_herdr_grid_restarts_the_observe_stream_and_hands_over_on_its_first_frame() {
    let mut fake = FakeHerdr::new();
    fake.script_terminal_stream(StreamKind::Control, PANE, [control().closed(REFUSED)]);
    fake.script_terminal_stream(
        StreamKind::Observe,
        PANE,
        [
            control().frame(FakeFrame::full(1, 60, 20, "old")),
            control()
                .wait_for_gate("restarted")
                .frame(FakeFrame::full(1, 50, 20, "new")),
        ],
    );
    let watched = Watched::new(&fake, RELEASE_GRACE);
    watched.show(size(80, 24), grid(60, 20));
    watched.wait_for_frame(b"old");

    watched.stream.set_herdr_grid(grid(50, 20));
    eventually(
        "the second observe stream",
        || runs_of(&fake, "observe"),
        |runs| runs.len() == 2,
    );
    fake.open_gate("restarted");

    let events = watched.wait_for_frame(b"new");
    let handed_over = frame(&events[2]);
    assert_eq!(
        (
            handed_over.width,
            handed_over.height,
            handed_over.bytes.as_slice()
        ),
        (50, 20, b"new".as_slice())
    );
    assert_eq!(
        runs_of(&fake, "observe")[1].args[4..],
        ["--cols", "50", "--rows", "20"]
    );
}

#[test]
fn the_control_channel_writes_to_the_session_that_holds_the_pane() {
    let mut fake = FakeHerdr::new();
    fake.script_terminal_stream(
        StreamKind::Control,
        PANE,
        [control().frame(FakeFrame::full(1, 80, 24, "x"))],
    );
    let watched = Watched::new(&fake, RELEASE_GRACE);
    watched.show(size(80, 24), grid(78, 22));
    watched.wait_for_frame(b"x");

    let line = json!({ "type": "terminal.input", "text": "ls\r" });
    watched.stream.control_channel().send(&line).expect("send");

    eventually(
        "the line to arrive",
        || fake.control_input(PANE),
        |input| input.contains(&line),
    );
}

/// A pane another client owns: control is refused, then observed, then a takeover is granted.
fn contested_pane(fake: &mut FakeHerdr) {
    fake.script_terminal_stream(
        StreamKind::Control,
        PANE,
        [
            control().closed(REFUSED),
            control().frame(FakeFrame::full(1, 80, 24, "mine")),
        ],
    );
    fake.script_terminal_stream(
        StreamKind::Observe,
        PANE,
        [control().frame(FakeFrame::full(1, 60, 20, "theirs"))],
    );
}

#[test]
fn the_first_input_forwarded_to_a_contested_pane_takes_it_with_takeover_and_arrives() {
    let mut fake = FakeHerdr::new();
    contested_pane(&mut fake);
    let watched = Watched::new(&fake, RELEASE_GRACE);
    watched.show(size(80, 24), grid(60, 20));
    watched.wait_for_frame(b"theirs");
    assert_eq!(runs_of(&fake, "control").len(), 1);

    watched.stream.forward(PaneInput::Text("ls\r".to_owned()));

    let events = watched.wait_for_frame(b"mine");
    assert_eq!(
        events[2],
        PaneStreamEvent::Ownership(PaneOwnership::TookControl)
    );
    assert_eq!(frame(&events[3]).bytes, b"mine");
    let runs = runs_of(&fake, "control");
    assert_eq!(runs.len(), 2);
    assert_eq!(
        runs[1].args,
        [
            "terminal",
            "session",
            "control",
            PANE,
            "--cols",
            "80",
            "--rows",
            "24",
            "--takeover"
        ]
    );
    wait_for_control_input(
        &fake,
        &[
            resize_line(size(80, 24)),
            resize_line(size(80, 24)),
            json!({ "type": "terminal.input", "text": "ls\r" }),
        ],
    );
}

#[test]
fn showing_resizing_and_regridding_a_contested_pane_never_takes_it() {
    let mut fake = FakeHerdr::new();
    contested_pane(&mut fake);
    let watched = Watched::new(&fake, RELEASE_GRACE);
    watched.show(size(80, 24), grid(60, 20));
    watched.wait_for_frame(b"theirs");

    watched.stream.set_size(size(120, 40));
    watched.stream.set_herdr_grid(grid(50, 18));
    eventually(
        "the observe stream to be replaced",
        || runs_of(&fake, "observe"),
        |runs| runs.len() == 2,
    );

    assert_eq!(runs_of(&fake, "control").len(), 1);
    assert!(
        fake.invocations()
            .iter()
            .all(|run| !run.args.iter().any(|arg| arg == "--takeover"))
    );
}

#[test]
fn scroll_and_mouse_reach_the_control_session_as_terminal_commands() {
    let mut fake = FakeHerdr::new();
    fake.script_terminal_stream(
        StreamKind::Control,
        PANE,
        [control().frame(FakeFrame::full(1, 80, 24, "x"))],
    );
    let watched = Watched::new(&fake, RELEASE_GRACE);
    watched.show(size(80, 24), grid(78, 22));
    watched.wait_for_frame(b"x");

    watched.stream.forward(PaneInput::Scroll {
        direction: ScrollDirection::Up,
        lines: 3,
        column: 4,
        row: 5,
    });
    watched.stream.forward(PaneInput::Mouse {
        action: MouseAction::Down,
        column: 6,
        row: 7,
    });

    wait_for_control_input(
        &fake,
        &[
            resize_line(size(80, 24)),
            json!({
                "type": "terminal.scroll",
                "direction": "up",
                "lines": 3,
                "source": "wheel",
                "column": 4,
                "row": 5
            }),
            json!({
                "type": "terminal.mouse",
                "action": "down",
                "button": "left",
                "column": 6,
                "row": 7
            }),
        ],
    );
    assert_eq!(runs_of(&fake, "control").len(), 1);
}

#[test]
fn named_keys_go_through_herdrs_mode_aware_send_keys_and_also_take_a_contested_pane() {
    let mut fake = FakeHerdr::new();
    contested_pane(&mut fake);
    let watched = Watched::new(&fake, RELEASE_GRACE);
    watched.show(size(80, 24), grid(60, 20));
    watched.wait_for_frame(b"theirs");

    watched.stream.forward(PaneInput::Key("up".to_owned()));
    let sent = eventually(
        "send-keys to run",
        || {
            fake.invocations()
                .into_iter()
                .filter(|run| run.args.get(1).map(String::as_str) == Some("send-keys"))
                .collect::<Vec<_>>()
        },
        |runs| !runs.is_empty(),
    );
    assert_eq!(sent[0].args, ["pane", "send-keys", PANE, "up"]);
    assert_eq!(sent[0].env("HERDR_SESSION"), Some("default"));
    eventually(
        "the takeover run to be logged",
        || runs_of(&fake, "control"),
        |runs| runs.len() == 2,
    );
    wait_for_control_input(
        &fake,
        &[resize_line(size(80, 24)), resize_line(size(80, 24))],
    );
}

#[test]
fn input_to_a_pane_warpr_is_not_holding_is_dropped() {
    let mut fake = FakeHerdr::new();
    fake.script_terminal_stream(
        StreamKind::Control,
        PANE,
        [control()
            .frame(FakeFrame::full(1, 80, 24, "x"))
            .closed(EVICTED)],
    );
    let watched = Watched::new(&fake, RELEASE_GRACE);

    watched.stream.forward(PaneInput::Text("hidden".to_owned()));
    watched.stream.forward(PaneInput::Key("enter".to_owned()));
    watched.stop();

    assert!(fake.invocations().is_empty());
}

#[test]
fn a_herdr_that_cannot_be_run_leaves_the_pane_unavailable() {
    let fake = FakeHerdr::new();
    let watched = Watched::with_cli(
        HerdrCli::new("/nonexistent/herdr"),
        &fake,
        PANE,
        RELEASE_GRACE,
    );

    watched.show(size(80, 24), grid(78, 22));

    let events = watched.wait_for("the pane to be unavailable", |event| {
        matches!(
            event,
            PaneStreamEvent::Ownership(PaneOwnership::Unavailable(_))
        )
    });
    let PaneStreamEvent::Ownership(PaneOwnership::Unavailable(reason)) = &events[0] else {
        panic!("expected unavailable, got {events:?}");
    };
    assert!(reason.contains("could not run herdr"), "{reason}");
}
