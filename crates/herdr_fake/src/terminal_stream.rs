use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::json;

/// Which `herdr terminal session` subcommand a script stands in for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamKind {
    /// Reads `terminal.*` lines on stdin and holds the pane's owner slot.
    Control,
    /// Streams frames at the requested viewport and never reads stdin.
    Observe,
}

impl StreamKind {
    pub(crate) fn subcommand(self) -> &'static str {
        match self {
            Self::Control => "control",
            Self::Observe => "observe",
        }
    }
}

/// One `terminal.frame` record.
#[derive(Clone, Debug)]
pub struct FakeFrame {
    seq: u64,
    full: bool,
    width: u16,
    height: u16,
    bytes: Vec<u8>,
}

impl FakeFrame {
    /// A frame that repaints every cell, as the first frame of a stream always does.
    pub fn full(seq: u64, width: u16, height: u16, bytes: impl AsRef<[u8]>) -> Self {
        Self::new(seq, true, width, height, bytes)
    }

    /// A frame that is a cursor-positioned diff against the previous one.
    pub fn incremental(seq: u64, width: u16, height: u16, bytes: impl AsRef<[u8]>) -> Self {
        Self::new(seq, false, width, height, bytes)
    }

    fn new(seq: u64, full: bool, width: u16, height: u16, bytes: impl AsRef<[u8]>) -> Self {
        Self {
            seq,
            full,
            width,
            height,
            bytes: bytes.as_ref().to_vec(),
        }
    }

    fn line(&self) -> String {
        json!({
            "type": "terminal.frame",
            "seq": self.seq,
            "full": self.full,
            "width": self.width,
            "height": self.height,
            "encoding": "ansi",
            "bytes": STANDARD.encode(&self.bytes),
        })
        .to_string()
    }
}

#[derive(Clone, Debug)]
enum Step {
    Print(String),
    Gate(String),
    InputLines(usize),
    Exit(i32),
}

/// What one run of a scripted `terminal session` command does.
///
/// Steps run in order. A run that is not ended by [`Self::closed`] or [`Self::exit_silently`]
/// keeps streaming until it is stopped: an observe run until it is killed, a control run until its
/// stdin closes, which it answers with a `detached` `terminal.closed` as Herdr does.
#[derive(Clone, Debug, Default)]
pub struct TerminalStreamScript {
    steps: Vec<Step>,
}

impl TerminalStreamScript {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn frame(mut self, frame: FakeFrame) -> Self {
        self.steps.push(Step::Print(frame.line()));
        self
    }

    /// Waits until the test calls [`crate::FakeHerdr::open_gate`] with `name`.
    pub fn wait_for_gate(mut self, name: &str) -> Self {
        self.steps.push(Step::Gate(name.to_owned()));
        self
    }

    /// Waits until this run has received `count` lines on stdin.
    pub fn wait_for_input_lines(mut self, count: usize) -> Self {
        self.steps.push(Step::InputLines(count));
        self
    }

    /// Prints a `terminal.closed` record with `reason` and exits 0.
    pub fn closed(mut self, reason: &str) -> Self {
        self.steps.push(Step::Print(
            json!({ "type": "terminal.closed", "reason": reason }).to_string(),
        ));
        self.steps.push(Step::Exit(0));
        self
    }

    /// Exits with `code` without a `terminal.closed` record, as a failed connection does.
    pub fn exit_silently(mut self, code: i32) -> Self {
        self.steps.push(Step::Exit(code));
        self
    }
}

pub(crate) struct StreamRuns {
    pub(crate) kind: StreamKind,
    pub(crate) runs: Vec<TerminalStreamScript>,
}

impl StreamRuns {
    /// The shell for the command at `index`. The `n`th run of the command follows the `n`th
    /// script, and the last script repeats.
    pub(crate) fn render(&self, index: usize) -> String {
        let mut script = format!(
            "mkdir -p \"$dir/streams\" \"$dir/gates\"\n\
             counter=\"$dir/streams/{index}.runs\"\n\
             run=$(cat \"$counter\" 2>/dev/null || echo 0)\n\
             echo $((run + 1)) > \"$counter\"\n\
             input=\"$dir/streams/{index}.$run.stdin\"\n\
             eof=\"$dir/streams/{index}.$run.eof\"\n"
        );
        if self.kind == StreamKind::Control {
            // Asynchronous commands read /dev/null in a non-interactive shell, so stdin is
            // handed to the recorder through another descriptor. Its stdout is closed so that
            // it does not keep the stream's pipe open after the script exits.
            script.push_str(
                "exec 3<&0\n: > \"$input\"\n\
                 ( cat <&3 >> \"$input\"; : > \"$eof\" ) > /dev/null 2>&1 &\nexec 3<&-\n",
            );
        }
        script.push_str("case \"$run\" in\n");
        let last = self.runs.len().saturating_sub(1);
        for (number, run) in self.runs.iter().enumerate() {
            let pattern = if number == last {
                "*".to_owned()
            } else {
                number.to_string()
            };
            script.push_str(&format!("{pattern})\n"));
            script.push_str(&self.render_run(run));
            script.push_str(";;\n");
        }
        script.push_str("esac\n");
        script
    }

    fn render_run(&self, run: &TerminalStreamScript) -> String {
        let mut script = String::new();
        for step in &run.steps {
            match step {
                Step::Print(line) => {
                    script.push_str(&format!("printf '%s\\n' {}\n", shell_quote(line)));
                }
                Step::Gate(name) => script.push_str(&format!(
                    "while [ ! -e \"$dir/gates/\"{} ]; do sleep 0.02; done\n",
                    shell_quote(name)
                )),
                Step::InputLines(count) => script.push_str(&format!(
                    "while [ \"$(wc -l < \"$input\" | tr -d ' ')\" -lt {count} ]; do sleep 0.02; done\n"
                )),
                Step::Exit(code) => script.push_str(&format!("exit {code}\n")),
            }
        }
        match self.kind {
            StreamKind::Control => {
                script.push_str("while [ ! -e \"$eof\" ]; do sleep 0.02; done\n");
                let closed = json!({ "type": "terminal.closed", "reason": "detached" });
                script.push_str(&format!(
                    "printf '%s\\n' {}\nexit 0\n",
                    shell_quote(&closed.to_string())
                ));
            }
            StreamKind::Observe => script.push_str("while :; do sleep 0.05; done\n"),
        }
        script
    }
}

/// The lines `terminal session control` runs of command `index` received on stdin, oldest first.
pub(crate) fn recorded_input(dir: &Path, index: usize) -> Vec<serde_json::Value> {
    let streams = dir.join("streams");
    let runs: usize = std::fs::read_to_string(streams.join(format!("{index}.runs")))
        .ok()
        .and_then(|runs| runs.trim().parse().ok())
        .unwrap_or(0);
    (0..runs)
        .flat_map(|run| {
            std::fs::read_to_string(streams.join(format!("{index}.{run}.stdin")))
                .unwrap_or_default()
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| serde_json::from_str(line).expect("recorded input is JSON"))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
