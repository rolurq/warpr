//! A pane's live terminal stream.
//!
//! [`PaneStream`] runs `herdr terminal session control` for a visible pane at the size warpr
//! chooses and falls back to `observe` when another client owns the pane. The decisions live in
//! [`crate::pane_machine`]; this module runs the subprocesses and reports what they print.

use std::io::{self, BufRead, BufReader, Write};
use std::mem;
use std::process::{Child, ChildStdin, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use instant::Instant;
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::pane_machine::{Action, Closure, Input, Machine, Timing};
pub use crate::pane_machine::{Contest, PaneOwnership};
pub use crate::pane_size::{GridSize, PaneSize};
use crate::{HerdrCli, PaneId, SessionTarget};

/// Resizes reach the pane at most this often; see ADR-0003 for the measurements behind it.
pub const RESIZE_INTERVAL: Duration = Duration::from_millis(100);

/// How long a pane may stay out of view before warpr lets go of it.
pub const RELEASE_GRACE: Duration = Duration::from_secs(10);

/// How long a released control session may take to exit before it is killed.
const REAP_LIMIT: Duration = Duration::from_secs(2);

/// One `terminal.frame` record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneFrame {
    pub seq: u64,
    /// Whether the frame repaints every cell. The first frame of a stream is, and any later one
    /// may be.
    pub full: bool,
    /// The grid the frame was drawn for.
    pub width: u16,
    pub height: u16,
    /// ANSI bytes to feed to a terminal.
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneStreamEvent {
    Frame(PaneFrame),
    Ownership(PaneOwnership),
}

/// The stdin of a pane's control session, which takes `terminal.*` JSON lines.
///
/// It stays the same handle as the session is replaced, and is empty while warpr holds none.
#[derive(Clone, Default)]
pub struct ControlChannel {
    stdin: Arc<Mutex<Option<ChildStdin>>>,
}

impl ControlChannel {
    /// Writes one command as a line. Fails with [`io::ErrorKind::NotConnected`] while warpr does
    /// not hold the pane.
    pub fn send(&self, command: &Value) -> io::Result<()> {
        let mut stdin = self.stdin.lock();
        let stdin = stdin
            .as_mut()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotConnected))?;
        writeln!(stdin, "{command}")?;
        stdin.flush()
    }

    fn attach(&self, stdin: ChildStdin) {
        *self.stdin.lock() = Some(stdin);
    }

    fn close(&self) {
        self.stdin.lock().take();
    }
}

/// Keeps one pane's stream running while the pane is visible, on threads of its own.
///
/// Dropping it releases the pane.
pub struct PaneStream {
    messages: Sender<Message>,
    control: ControlChannel,
    worker: Option<JoinHandle<()>>,
}

impl PaneStream {
    /// Starts managing `pane` of `target`. Nothing runs until the pane is shown with
    /// [`Self::set_visible`] and given a size with [`Self::set_size`].
    ///
    /// `on_event` runs on the stream's own thread.
    pub fn start(
        cli: HerdrCli,
        target: SessionTarget,
        pane: PaneId,
        on_event: impl FnMut(PaneStreamEvent) + Send + 'static,
    ) -> Self {
        Self::start_with_timing(
            cli,
            target,
            pane,
            Timing {
                resize_interval: RESIZE_INTERVAL,
                release_grace: RELEASE_GRACE,
            },
            on_event,
        )
    }

    pub(crate) fn start_with_timing(
        cli: HerdrCli,
        target: SessionTarget,
        pane: PaneId,
        timing: Timing,
        on_event: impl FnMut(PaneStreamEvent) + Send + 'static,
    ) -> Self {
        let (messages, inbox) = mpsc::channel();
        let control = ControlChannel::default();
        let worker = Worker {
            cli,
            target,
            pane,
            machine: Machine::new(timing),
            control: control.clone(),
            controlling: None,
            observing: None,
            replacing: None,
            next_process: 0,
            to_self: messages.clone(),
            on_event: Box::new(on_event),
        };
        let worker = thread::spawn(move || worker.run(inbox));
        Self {
            messages,
            control,
            worker: Some(worker),
        }
    }

    /// Sets whether the pane is on screen. A pane that stays hidden for [`RELEASE_GRACE`] is
    /// released, and acquired again when shown.
    pub fn set_visible(&self, visible: bool) {
        self.send(Input::Visible(visible));
    }

    /// Sets the size the pane should have while warpr controls it. Changes reach the pane through
    /// a throttle, snapped to whole cells by [`PaneSize::fit`].
    pub fn set_size(&self, size: PaneSize) {
        self.send(Input::Size(size));
    }

    /// Sets the grid Herdr itself lays the pane out at, which a contested pane is watched at.
    pub fn set_herdr_grid(&self, grid: GridSize) {
        self.send(Input::Grid(grid));
    }

    /// The stdin of the control session, for forwarding input to the pane.
    pub fn control_channel(&self) -> ControlChannel {
        self.control.clone()
    }

    fn send(&self, input: Input) {
        let _ = self.messages.send(Message::Command(input));
    }
}

impl Drop for PaneStream {
    fn drop(&mut self) {
        let _ = self.messages.send(Message::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

enum Message {
    Command(Input),
    Frame { process: u64, frame: PaneFrame },
    Ended { process: u64, closure: Closure },
    Shutdown,
}

#[derive(Clone, Copy)]
enum Kind {
    Control,
    Observe,
}

struct Process {
    id: u64,
    child: Child,
}

struct Worker {
    cli: HerdrCli,
    target: SessionTarget,
    pane: PaneId,
    machine: Machine,
    control: ControlChannel,
    controlling: Option<Process>,
    observing: Option<Process>,
    /// An observe stream started to replace `observing`, which keeps feeding until the
    /// replacement has produced a frame.
    replacing: Option<Process>,
    next_process: u64,
    to_self: Sender<Message>,
    on_event: Box<dyn FnMut(PaneStreamEvent) + Send>,
}

impl Worker {
    fn run(mut self, inbox: Receiver<Message>) {
        loop {
            let message = match self.machine.next_deadline() {
                Some(deadline) => {
                    match inbox.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                        Ok(message) => Some(message),
                        Err(RecvTimeoutError::Timeout) => None,
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
                None => match inbox.recv() {
                    Ok(message) => Some(message),
                    Err(_) => break,
                },
            };
            let input = match message {
                Some(Message::Command(input)) => input,
                Some(Message::Frame { process, frame }) => {
                    self.frame(process, frame);
                    continue;
                }
                Some(Message::Ended { process, closure }) => match self.ended(process, closure) {
                    Some(input) => input,
                    None => continue,
                },
                Some(Message::Shutdown) => break,
                None => Input::Tick,
            };
            self.apply(input);
        }
        self.stop_everything();
    }

    /// Feeds `input` to the machine and carries out what it asks for, including the follow-up
    /// inputs that carrying it out causes.
    fn apply(&mut self, input: Input) {
        let mut inputs = vec![input];
        while let Some(input) = inputs.pop() {
            for action in self.machine.handle(Instant::now(), input) {
                if let Some(follow_up) = self.perform(action) {
                    inputs.push(follow_up);
                }
            }
        }
    }

    fn perform(&mut self, action: Action) -> Option<Input> {
        match action {
            Action::SpawnControl(size) => {
                let cols = size.cols.to_string();
                let rows = size.rows.to_string();
                match self.spawn(Kind::Control, &cols, &rows) {
                    Ok(process) => self.controlling = Some(process),
                    Err(error) => return Some(Input::ControlClosed(spawn_failure(&error))),
                }
            }
            Action::SendResize(size) => {
                // A failed write means the session already ended, which its `terminal.closed`
                // reports.
                let _ = self.control.send(&json!({
                    "type": "terminal.resize",
                    "cols": size.cols,
                    "rows": size.rows,
                    "cell_width_px": size.cell_width_px,
                    "cell_height_px": size.cell_height_px,
                }));
            }
            Action::ReleaseControl => self.release_control(),
            Action::SpawnObserve(grid) => {
                self.stop_observing();
                match self.spawn(
                    Kind::Observe,
                    &grid.cols.to_string(),
                    &grid.rows.to_string(),
                ) {
                    Ok(process) => self.observing = Some(process),
                    Err(error) => return Some(Input::ObserveClosed(spawn_failure(&error))),
                }
            }
            Action::ReplaceObserve(grid) => {
                if let Some(stale) = self.replacing.take() {
                    kill(stale);
                }
                match self.spawn(
                    Kind::Observe,
                    &grid.cols.to_string(),
                    &grid.rows.to_string(),
                ) {
                    Ok(process) => self.replacing = Some(process),
                    Err(error) => return Some(Input::ObserveClosed(spawn_failure(&error))),
                }
            }
            Action::StopObserve => self.stop_observing(),
            Action::Publish(ownership) => (self.on_event)(PaneStreamEvent::Ownership(ownership)),
        }
        None
    }

    fn spawn(&mut self, kind: Kind, cols: &str, rows: &str) -> io::Result<Process> {
        let subcommand = match kind {
            Kind::Control => "control",
            Kind::Observe => "observe",
        };
        let mut command = self.cli.command(
            &self.target,
            &[
                "terminal",
                "session",
                subcommand,
                self.pane.as_str(),
                "--cols",
                cols,
                "--rows",
                rows,
            ],
        );
        command.stdout(Stdio::piped()).stderr(Stdio::null());
        command.stdin(match kind {
            Kind::Control => Stdio::piped(),
            Kind::Observe => Stdio::null(),
        });
        let mut child = command.spawn()?;
        let stdout = child.stdout.take().expect("stdout is piped");
        if let Some(stdin) = child.stdin.take() {
            self.control.attach(stdin);
        }
        let id = self.next_process;
        self.next_process += 1;
        spawn_reader(id, stdout, self.to_self.clone());
        Ok(Process { id, child })
    }

    fn frame(&mut self, process: u64, frame: PaneFrame) {
        if self.replacing.as_ref().is_some_and(|new| new.id == process) {
            let replacement = self.replacing.take();
            if let Some(old) = mem::replace(&mut self.observing, replacement) {
                kill(old);
            }
        } else if self
            .controlling
            .as_ref()
            .is_some_and(|live| live.id == process)
        {
            // Ownership is published before the frame that proves it.
            self.apply(Input::ControlFrame);
        } else if self
            .observing
            .as_ref()
            .is_none_or(|live| live.id != process)
        {
            return;
        }
        (self.on_event)(PaneStreamEvent::Frame(frame));
    }

    fn ended(&mut self, process: u64, closure: Closure) -> Option<Input> {
        if self
            .controlling
            .as_ref()
            .is_some_and(|live| live.id == process)
        {
            if let Some(finished) = self.controlling.take() {
                reap(finished);
            }
            self.control.close();
            return Some(Input::ControlClosed(closure));
        }
        let observing_ended = self
            .observing
            .as_ref()
            .is_some_and(|live| live.id == process);
        let replacing_ended = self
            .replacing
            .as_ref()
            .is_some_and(|live| live.id == process);
        (observing_ended || replacing_ended).then_some(Input::ObserveClosed(closure))
    }

    fn release_control(&mut self) {
        let _ = self.control.send(&json!({ "type": "terminal.release" }));
        self.control.close();
        if let Some(process) = self.controlling.take() {
            reap(process);
        }
    }

    fn stop_observing(&mut self) {
        for process in [self.observing.take(), self.replacing.take()]
            .into_iter()
            .flatten()
        {
            kill(process);
        }
    }

    fn stop_everything(&mut self) {
        self.release_control();
        self.stop_observing();
    }
}

fn spawn_failure(error: &io::Error) -> Closure {
    Closure::Other(format!("could not run herdr: {error}"))
}

fn kill(mut process: Process) {
    let _ = process.child.kill();
    let _ = process.child.wait();
}

/// Waits for a released control session to exit on its own, killing it when it does not.
fn reap(mut process: Process) {
    thread::spawn(move || {
        let deadline = Instant::now() + REAP_LIMIT;
        while Instant::now() < deadline {
            if !matches!(process.child.try_wait(), Ok(None)) {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let _ = process.child.kill();
        let _ = process.child.wait();
    });
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum Record {
    #[serde(rename = "terminal.frame")]
    Frame {
        seq: u64,
        full: bool,
        width: u16,
        height: u16,
        bytes: String,
    },
    #[serde(rename = "terminal.closed")]
    Closed { reason: Option<String> },
    #[serde(other)]
    Other,
}

/// Reads a stream's stdout until it ends, then reports how it ended.
fn spawn_reader(process: u64, stdout: impl io::Read + Send + 'static, messages: Sender<Message>) {
    thread::spawn(move || {
        let mut reason = None;
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            match serde_json::from_str::<Record>(&line) {
                Ok(Record::Frame {
                    seq,
                    full,
                    width,
                    height,
                    bytes,
                }) => {
                    let Ok(bytes) = STANDARD.decode(bytes) else {
                        continue;
                    };
                    let frame = PaneFrame {
                        seq,
                        full,
                        width,
                        height,
                        bytes,
                    };
                    if messages.send(Message::Frame { process, frame }).is_err() {
                        return;
                    }
                }
                Ok(Record::Closed { reason: closed }) => {
                    reason = Some(closed.unwrap_or_default());
                }
                Ok(Record::Other) | Err(_) => {}
            }
        }
        let closure = Closure::from_reason(reason.as_deref());
        let _ = messages.send(Message::Ended { process, closure });
    });
}

#[cfg(test)]
#[path = "pane_stream_tests.rs"]
mod tests;
