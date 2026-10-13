use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Environment variables the scripted executable reports having been started with.
const RECORDED_ENV: [&str; 5] = [
    "HERDR_SOCKET_PATH",
    "HERDR_SESSION",
    "HERDR_PANE_ID",
    "HERDR_TAB_ID",
    "HERDR_WORKSPACE_ID",
];

const UNSCRIPTED_EXIT_CODE: i32 = 64;

/// One run of the scripted `herdr` executable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invocation {
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
    exit_code: i32,
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
        let index = self.commands.len();
        fs::write(self.dir().join("stdout").join(index.to_string()), stdout)
            .expect("write scripted stdout");
        self.commands.push(ScriptedCommand {
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            exit_code,
        });
        self.write_script();
    }

    pub(crate) fn invocations(&self) -> Vec<Invocation> {
        let log = fs::read_to_string(self.dir().join("invocations.log")).unwrap_or_default();
        let mut invocations = Vec::new();
        let mut current: Option<Invocation> = None;
        for line in log.lines() {
            match (line, current.as_mut()) {
                ("begin", _) => {
                    current = Some(Invocation {
                        args: Vec::new(),
                        herdr_env: Vec::new(),
                    })
                }
                ("end", _) => invocations.extend(current.take()),
                (_, Some(invocation)) => {
                    if let Some(arg) = line.strip_prefix("arg:") {
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

    fn dir(&self) -> &Path {
        self.path.parent().expect("binary path has a parent")
    }

    /// Replaces the executable atomically: rewriting a script in place while another thread
    /// spawns it can fail with "text file busy".
    fn write_script(&self) {
        let dir = self.dir();
        let staged = dir.join("herdr.staged");
        let mut file = fs::File::create(&staged).expect("create fake herdr script");
        file.write_all(self.render(dir).as_bytes())
            .expect("write fake herdr script");
        file.set_permissions(fs::Permissions::from_mode(0o755))
            .expect("make fake herdr executable");
        drop(file);
        fs::rename(&staged, &self.path).expect("install fake herdr script");
    }

    fn render(&self, dir: &Path) -> String {
        let dir = shell_quote(&dir.to_string_lossy());
        let mut script = format!("#!/bin/sh\ndir={dir}\n{{\nprintf 'begin\\n'\n");
        script.push_str("for arg in \"$@\"; do printf 'arg:%s\\n' \"$arg\"; done\n");
        for var in RECORDED_ENV {
            script.push_str(&format!(
                "[ -n \"${{{var}+x}}\" ] && printf 'env:{var}=%s\\n' \"${var}\"\n"
            ));
        }
        script.push_str("printf 'end\\n'\n} >> \"$dir/invocations.log\"\n");

        for (index, command) in self.commands.iter().enumerate() {
            let mut condition = format!("[ \"$#\" -eq {} ]", command.args.len());
            for (position, arg) in command.args.iter().enumerate() {
                condition.push_str(&format!(
                    " && [ \"${}\" = {} ]",
                    position + 1,
                    shell_quote(arg)
                ));
            }
            script.push_str(&format!(
                "if {condition}; then cat \"$dir/stdout/{index}\"; exit {}; fi\n",
                command.exit_code
            ));
        }
        script.push_str(&format!(
            "echo \"fake herdr: unscripted invocation: $*\" >&2\nexit {UNSCRIPTED_EXIT_CODE}\n"
        ));
        script
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
