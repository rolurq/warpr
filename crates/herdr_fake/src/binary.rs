use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use command::blocking::Command;

use crate::terminal_stream::{StreamRuns, recorded_input};

/// Environment variables the scripted executable reports having been started with.
const RECORDED_ENV: [&str; 5] = [
    "HERDR_SOCKET_PATH",
    "HERDR_SESSION",
    "HERDR_PANE_ID",
    "HERDR_TAB_ID",
    "HERDR_WORKSPACE_ID",
];

const UNSCRIPTED_EXIT_CODE: i32 = 64;

/// Lives in the fake's directory, so long-running scripted commands notice the owning
/// [`crate::FakeHerdr`] being dropped, which deletes the directory, and exit.
const INVOCATION_LOG: &str = "invocations.log";

const RUNNING_MARKER: &str = "running";

/// One run of the scripted `herdr` executable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invocation {
    /// The process id of the run, which a long-running scripted command keeps for its lifetime.
    pub pid: u32,
    pub args: Vec<String>,
    /// The Herdr-related variables that were set, as `(name, value)`.
    pub herdr_env: Vec<(String, String)>,
}

impl Invocation {
    pub fn env(&self, name: &str) -> Option<&str> {
        self.herdr_env
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, value)| value.as_str())
    }
}

struct ScriptedCommand {
    args: Vec<String>,
    outcome: Outcome,
}

enum Outcome {
    Exit {
        exit_code: i32,
    },
    /// Never answers, like a Herdr whose socket accepts connections and stays silent.
    Hang,
    /// Keeps running, like a server, until the owning [`crate::FakeHerdr`] is dropped.
    UntilShutdown,
    /// Streams `terminal.*` records for any invocation that starts with the command's arguments.
    Stream(StreamRuns),
}

/// A `/bin/sh` script standing in for the `herdr` executable.
///
/// Its behaviour lives in the generated script, so it works from any process that is handed its
/// path, without the test binary having to build or locate a helper executable.
pub(crate) struct ScriptedBinary {
    path: PathBuf,
    commands: Vec<ScriptedCommand>,
}

impl ScriptedBinary {
    pub(crate) fn new(path: PathBuf) -> Self {
        let dir = path.parent().expect("binary path has a parent").to_owned();
        fs::create_dir_all(dir.join("stdout")).expect("create fake herdr directory");
        fs::write(dir.join(RUNNING_MARKER), "").expect("mark fake herdr as running");
        let binary = Self {
            path,
            commands: Vec::new(),
        };
        binary.write_script();
        binary
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn script(&mut self, args: &[&str], stdout: &str, exit_code: i32) {
        self.push(args, stdout, Outcome::Exit { exit_code });
    }

    pub(crate) fn script_hang(&mut self, args: &[&str]) {
        self.push(args, "", Outcome::Hang);
    }

    pub(crate) fn script_until_shutdown(&mut self, args: &[&str]) {
        self.push(args, "", Outcome::UntilShutdown);
    }

    /// Scripts every `herdr terminal session <kind> <pane> ...` invocation.
    pub(crate) fn script_stream(&mut self, pane: &str, runs: StreamRuns) {
        let args = ["terminal", "session", runs.kind.subcommand(), pane];
        self.push(&args, "", Outcome::Stream(runs));
    }

    /// Opens the gate `name` that scripted streams may be waiting on.
    pub(crate) fn open_gate(&self, name: &str) {
        let gates = self.dir().join("gates");
        fs::create_dir_all(&gates).expect("create gates directory");
        fs::write(gates.join(name), "").expect("open gate");
    }

    /// The `terminal.*` lines the control runs of `pane` received on stdin so far.
    pub(crate) fn control_input(&self, pane: &str) -> Vec<serde_json::Value> {
        let args = ["terminal", "session", "control", pane];
        self.commands
            .iter()
            .position(|command| command.args == args)
            .map(|index| recorded_input(self.dir(), index))
            .unwrap_or_default()
    }

    /// Scripts `args`, replacing what an earlier script of the same arguments did.
    fn push(&mut self, args: &[&str], stdout: &str, outcome: Outcome) {
        let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        let index = self
            .commands
            .iter()
            .position(|command| command.args == args)
            .unwrap_or(self.commands.len());
        fs::write(self.dir().join("stdout").join(index.to_string()), stdout)
            .expect("write scripted stdout");
        let command = ScriptedCommand { args, outcome };
        match self.commands.get_mut(index) {
            Some(existing) => *existing = command,
            None => self.commands.push(command),
        }
        self.write_script();
    }

    pub(crate) fn invocations(&self) -> Vec<Invocation> {
        read_invocations(&self.dir().join(INVOCATION_LOG))
    }

    pub(crate) fn invocation_log(&self) -> PathBuf {
        self.dir().join(INVOCATION_LOG)
    }

    fn dir(&self) -> &Path {
        self.path.parent().expect("binary path has a parent")
    }

    /// Replaces the executable atomically, so a run never starts a half-written script.
    ///
    /// A child process writes it: a file this process held open for writing would be inherited by
    /// every child another thread spawns meanwhile, and running the script while any of them still
    /// had it open fails with "text file busy".
    fn write_script(&self) {
        let dir = self.dir();
        let staged = dir.join("herdr.staged");
        let mut writer = Command::new("/bin/sh")
            .args(["-c", "cat > \"$1\" && chmod 755 \"$1\"", "sh"])
            .arg(&staged)
            .stdin(Stdio::piped())
            .spawn()
            .expect("start writing fake herdr script");
        let mut script = writer.stdin.take().expect("stdin is piped");
        script
            .write_all(self.render(dir).as_bytes())
            .expect("write fake herdr script");
        drop(script);
        let status = writer.wait().expect("finish writing fake herdr script");
        assert!(
            status.success(),
            "writing fake herdr script failed: {status}"
        );
        fs::rename(&staged, &self.path).expect("install fake herdr script");
    }

    fn render(&self, dir: &Path) -> String {
        let dir = shell_quote(&dir.to_string_lossy());
        // The record is staged and appended with a single `cat` write, so that concurrent
        // invocations cannot interleave their lines.
        let mut script = format!(
            "#!/bin/sh\ndir={dir}\nrecord=\"$dir/record.$$\"\n{{\nprintf 'begin\\n'\nprintf 'pid:%s\\n' \"$$\"\n"
        );
        script.push_str("for arg in \"$@\"; do printf 'arg:%s\\n' \"$arg\"; done\n");
        for var in RECORDED_ENV {
            script.push_str(&format!(
                "[ -n \"${{{var}+x}}\" ] && printf 'env:{var}=%s\\n' \"${var}\"\n"
            ));
        }
        script.push_str(&format!(
            "printf 'end\\n'\n}} > \"$record\"\ncat \"$record\" >> \"$dir/{INVOCATION_LOG}\"\nrm -f \"$record\"\n"
        ));

        for (index, command) in self.commands.iter().enumerate() {
            let comparison = match command.outcome {
                Outcome::Stream(_) => "-ge",
                Outcome::Exit { .. } | Outcome::Hang | Outcome::UntilShutdown => "-eq",
            };
            let mut condition = format!("[ \"$#\" {comparison} {} ]", command.args.len());
            for (position, arg) in command.args.iter().enumerate() {
                condition.push_str(&format!(
                    " && [ \"${}\" = {} ]",
                    position + 1,
                    shell_quote(arg)
                ));
            }
            let action = match &command.outcome {
                Outcome::Exit { exit_code } => {
                    format!("cat \"$dir/stdout/{index}\"; exit {exit_code}")
                }
                // `exec` so that killing the script kills the sleep too.
                Outcome::Hang => "exec sleep 600".to_owned(),
                Outcome::UntilShutdown => format!(
                    "while [ -e \"$dir/{RUNNING_MARKER}\" ]; do sleep 0.05; done; \
                     cat \"$dir/stdout/{index}\"; exit 0"
                ),
                Outcome::Stream(runs) => runs.render(index),
            };
            script.push_str(&format!("if {condition}; then\n{action}\nfi\n"));
        }
        script.push_str(&format!(
            "echo \"fake herdr: unscripted invocation: $*\" >&2\nexit {UNSCRIPTED_EXIT_CODE}\n"
        ));
        script
    }
}

/// Parses the log the scripted executable appends to on every run.
pub(crate) fn read_invocations(log: &Path) -> Vec<Invocation> {
    let log = fs::read_to_string(log).unwrap_or_default();
    let mut invocations = Vec::new();
    let mut current: Option<Invocation> = None;
    for line in log.lines() {
        match (line, current.as_mut()) {
            ("begin", _) => {
                current = Some(Invocation {
                    pid: 0,
                    args: Vec::new(),
                    herdr_env: Vec::new(),
                })
            }
            ("end", _) => invocations.extend(current.take()),
            (_, Some(invocation)) => {
                if let Some(pid) = line.strip_prefix("pid:") {
                    invocation.pid = pid.parse().unwrap_or_default();
                } else if let Some(arg) = line.strip_prefix("arg:") {
                    invocation.args.push(arg.to_owned());
                } else if let Some(var) = line.strip_prefix("env:")
                    && let Some((name, value)) = var.split_once('=')
                {
                    invocation
                        .herdr_env
                        .push((name.to_owned(), value.to_owned()));
                }
            }
            _ => {}
        }
    }
    invocations
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
