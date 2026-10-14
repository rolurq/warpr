use std::fmt::Display;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::{Output, Stdio};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::SessionTarget;
use crate::cli::{HERDR_BINARY_ENV, HerdrCli};

const POLL_INTERVAL: Duration = Duration::from_millis(10);

const INSTALL_URL: &str = "https://herdr.dev";

/// Why warpr declines to connect to a Herdr session it could otherwise reach.
///
/// Each message says what is wrong and what to do about it, so a front-end can show it as is.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error(
        "The `herdr` executable was not found at {}. Install Herdr ({INSTALL_URL}) so that \
         `herdr` is on your PATH, or set {HERDR_BINARY_ENV} to where it is installed.",
        .binary.display(),
        INSTALL_URL = INSTALL_URL,
        HERDR_BINARY_ENV = HERDR_BINARY_ENV
    )]
    ExecutableMissing { binary: PathBuf },
    #[error(
        "The `herdr` executable at {} cannot be run: {reason}. Reinstall Herdr ({INSTALL_URL}), \
         or point {HERDR_BINARY_ENV} at a working copy.",
        .binary.display(),
        INSTALL_URL = INSTALL_URL,
        HERDR_BINARY_ENV = HERDR_BINARY_ENV
    )]
    ExecutableNotRunnable { binary: PathBuf, reason: String },
    /// The executable ran, but its answer cannot be trusted to mean the session is compatible.
    #[error(
        "Herdr's compatibility could not be checked: {detail}. warpr will not connect to a Herdr \
         it cannot verify."
    )]
    StatusUnavailable { detail: String },
    #[error(
        "Herdr's endpoint is incompatible: the running server (version {}) reports endpoint \
         protocol generation {}, but warpr needs generation {needed}, the one spoken by the \
         installed `herdr` {executable_version}. Make the server and the `herdr` executable the \
         same Herdr release, then restart the session's server.",
        or_unknown(.server_version),
        or_unknown(.reported)
    )]
    IncompatibleEndpoint {
        executable_version: String,
        needed: u32,
        server_version: Option<String>,
        reported: Option<u32>,
    },
    #[error(
        "Herdr's private protocol is incompatible: the running server (version {}) speaks \
         protocol {}, but warpr needs protocol {needed}, the one spoken by the installed `herdr` \
         {executable_version}. Make the server and the `herdr` executable the same Herdr release, \
         then restart the session's server.",
        or_unknown(.server_version),
        or_unknown(.reported)
    )]
    IncompatibleProtocol {
        executable_version: String,
        needed: u32,
        server_version: Option<String>,
        reported: Option<u32>,
    },
}

fn or_unknown<T: Display>(value: &Option<T>) -> String {
    value
        .as_ref()
        .map_or_else(|| "unknown".to_owned(), ToString::to_string)
}

/// The part of `herdr status server --json` this check reads. Herdr reports a compatibility
/// verdict as `null` when the server did not say enough to give one.
#[derive(Deserialize)]
struct ServerStatus {
    running: bool,
    version: Option<String>,
    protocol: Option<u32>,
    capabilities: Option<ServerCapabilities>,
    compatible: Option<bool>,
    endpoint_compatible: Option<bool>,
}

#[derive(Deserialize)]
struct ServerCapabilities {
    endpoint_protocol_generation: Option<u32>,
}

/// The part of `herdr status client --json` this check reads: what the installed executable
/// speaks, which is what the running server has to match.
#[derive(Deserialize)]
struct ClientStatus {
    version: String,
    protocol: u32,
    endpoint_protocol_generation: u32,
}

/// Checks that the `herdr` executable runs and that the server of `target`, if one is running, is
/// compatible with it. Each `herdr` run is abandoned after `timeout`.
///
/// A session whose server is not running passes: there is nothing to be incompatible with, and
/// reaching its socket reports the missing server.
pub(crate) fn check(
    cli: &HerdrCli,
    target: &SessionTarget,
    timeout: Duration,
) -> Result<(), Refusal> {
    let server: ServerStatus = status(cli, target, "server", timeout)?;
    if !server.running {
        return Ok(());
    }

    let endpoint_compatible = server.endpoint_compatible == Some(true);
    let protocol_compatible = server.compatible == Some(true);
    if endpoint_compatible && protocol_compatible {
        return Ok(());
    }

    let client: ClientStatus = status(cli, target, "client", timeout)?;
    if !endpoint_compatible {
        return Err(Refusal::IncompatibleEndpoint {
            executable_version: client.version,
            needed: client.endpoint_protocol_generation,
            server_version: server.version,
            reported: server
                .capabilities
                .and_then(|capabilities| capabilities.endpoint_protocol_generation),
        });
    }
    Err(Refusal::IncompatibleProtocol {
        executable_version: client.version,
        needed: client.protocol,
        server_version: server.version,
        reported: server.protocol,
    })
}

fn status<T: DeserializeOwned>(
    cli: &HerdrCli,
    target: &SessionTarget,
    scope: &str,
    timeout: Duration,
) -> Result<T, Refusal> {
    let command = format!("herdr status {scope}");
    let output = run_bounded(cli, target, &["status", scope, "--json"], timeout).map_err(
        |error| match error.kind() {
            io::ErrorKind::NotFound => Refusal::ExecutableMissing {
                binary: cli.binary().to_owned(),
            },
            io::ErrorKind::TimedOut => Refusal::StatusUnavailable {
                detail: format!("`{command}` did not finish within {timeout:?}"),
            },
            _ => Refusal::ExecutableNotRunnable {
                binary: cli.binary().to_owned(),
                reason: error.to_string(),
            },
        },
    )?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        let detail = if stderr.is_empty() {
            format!("`{command}` exited with {}", output.status)
        } else {
            format!("`{command}` exited with {}: {stderr}", output.status)
        };
        return Err(Refusal::StatusUnavailable { detail });
    }

    serde_json::from_slice(&output.stdout).map_err(|error| Refusal::StatusUnavailable {
        detail: format!("the output of `{command}` was not understood: {error}"),
    })
}

/// Runs `herdr <args>`, killing it and failing with [`io::ErrorKind::TimedOut`] when it has not
/// exited within `timeout`. `herdr` has no timeout of its own: one that reaches a socket nothing
/// answers on waits for as long as that socket stays open.
fn run_bounded(
    cli: &HerdrCli,
    target: &SessionTarget,
    args: &[&str],
    timeout: Duration,
) -> io::Result<Output> {
    let mut child = cli
        .command(target, args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    // Both pipes are drained while waiting, or a child filling one would never exit.
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());

    // Instant is not available on every target this workspace builds for.
    let mut waited = Duration::ZERO;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if waited >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::ErrorKind::TimedOut.into());
        }
        thread::sleep(POLL_INTERVAL);
        waited += POLL_INTERVAL;
    };
    Ok(Output {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

fn drain(pipe: Option<impl Read + Send + 'static>) -> JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut bytes);
        }
        bytes
    })
}

#[cfg(test)]
#[path = "compat_tests.rs"]
mod tests;
