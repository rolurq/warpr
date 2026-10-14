//! The long-lived `events.subscribe` connection.
//!
//! Events are only ever read as "something changed": their payloads are never decoded, because
//! Herdr gives events and snapshots no shared ordering, so replaying a payload onto a snapshot
//! can regress state.

use std::io::{BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::{ConnectionFailure, socket};

const REQUEST_ID: &str = "warpr-events";

const EVENTS_LOST: &str = "events_lost";

/// Subscription types for every event that can change what a snapshot reports.
///
/// `pane.agent_status_changed`, `pane.output_matched` and `pane.scroll_changed` are left out
/// because Herdr only accepts them for one named pane.
const LIFECYCLE_SUBSCRIPTIONS: [&str; 24] = [
    "workspace.created",
    "workspace.updated",
    "workspace.metadata_updated",
    "workspace.renamed",
    "workspace.moved",
    "workspace.reordered",
    "workspace.closed",
    "workspace.focused",
    "worktree.created",
    "worktree.opened",
    "worktree.removed",
    "tab.created",
    "tab.closed",
    "tab.focused",
    "tab.renamed",
    "tab.moved",
    "pane.created",
    "pane.closed",
    "pane.updated",
    "pane.focused",
    "pane.moved",
    "pane.exited",
    "pane.agent_detected",
    "layout.updated",
];

pub(crate) fn lifecycle_subscriptions() -> Vec<Value> {
    LIFECYCLE_SUBSCRIPTIONS
        .iter()
        .map(|event| json!({ "type": event }))
        .collect()
}

/// Why a subscription stopped delivering events.
#[derive(Debug)]
pub(crate) enum StreamEnd {
    /// Herdr dropped events this subscription had not read and closed it. The server is healthy,
    /// but whatever was read from it before is stale.
    EventsLost,
    Failed(ConnectionFailure),
}

impl StreamEnd {
    pub(crate) fn into_failure(self) -> ConnectionFailure {
        match self {
            Self::EventsLost => ConnectionFailure::Rejected {
                code: EVENTS_LOST.to_owned(),
                message: "the event subscription fell behind Herdr's retained history".to_owned(),
            },
            Self::Failed(failure) => failure,
        }
    }
}

/// Closes the subscription's socket from another thread, which releases a reader blocked in
/// [`EventStream::wait_for_event`].
pub(crate) struct StreamShutdown(UnixStream);

impl StreamShutdown {
    pub(crate) fn shutdown(&self) {
        let _ = self.0.shutdown(Shutdown::Both);
    }
}

#[derive(Deserialize)]
struct ErrorBody {
    code: String,
    message: String,
}

enum Frame {
    Success(Value),
    Failure(ErrorBody),
    /// Anything else, including shapes this client does not know: all of it means Herdr has
    /// something to say, which is enough to invalidate.
    Event,
}

pub(crate) struct EventStream {
    socket_path: PathBuf,
    reader: BufReader<UnixStream>,
}

impl EventStream {
    /// Subscribes to Herdr's lifecycle events and returns once Herdr has started the
    /// subscription, so a snapshot read after this returns cannot miss a later change.
    pub(crate) fn subscribe(socket_path: &Path) -> Result<(Self, StreamShutdown), StreamEnd> {
        Self::subscribe_to(socket_path, lifecycle_subscriptions())
    }

    /// Like [`Self::subscribe`], to the given `events.subscribe` entries.
    pub(crate) fn subscribe_to(
        socket_path: &Path,
        subscriptions: Vec<Value>,
    ) -> Result<(Self, StreamShutdown), StreamEnd> {
        let io_failed = |error| StreamEnd::Failed(socket::io_failure(socket_path, error));

        let stream = socket::connect(socket_path).map_err(StreamEnd::Failed)?;
        let shutdown = stream.try_clone().map_err(io_failed)?;

        let request = json!({
            "id": REQUEST_ID,
            "method": "events.subscribe",
            "params": { "subscriptions": subscriptions },
        });
        writeln!(&stream, "{request}").map_err(io_failed)?;

        let mut events = Self {
            socket_path: socket_path.to_owned(),
            reader: BufReader::new(stream),
        };
        match events.read_frame()? {
            Frame::Success(result) if result["type"] == "subscription_started" => {}
            Frame::Success(result) => {
                return Err(StreamEnd::Failed(ConnectionFailure::Protocol(format!(
                    "expected subscription_started, got {result}"
                ))));
            }
            Frame::Failure(error) => return Err(Self::rejection(error)),
            Frame::Event => {
                return Err(StreamEnd::Failed(ConnectionFailure::Protocol(
                    "an event arrived before the subscription was acknowledged".to_owned(),
                )));
            }
        }

        // Events can be hours apart, so a quiet stream is not a failure.
        events
            .reader
            .get_ref()
            .set_read_timeout(None)
            .map_err(io_failed)?;
        Ok((events, StreamShutdown(shutdown)))
    }

    /// Blocks until Herdr pushes something, which invalidates whatever was read before.
    pub(crate) fn wait_for_event(&mut self) -> Result<(), StreamEnd> {
        match self.read_frame()? {
            Frame::Failure(error) => Err(Self::rejection(error)),
            Frame::Success(_) | Frame::Event => Ok(()),
        }
    }

    fn rejection(error: ErrorBody) -> StreamEnd {
        if error.code == EVENTS_LOST {
            StreamEnd::EventsLost
        } else {
            StreamEnd::Failed(ConnectionFailure::Rejected {
                code: error.code,
                message: error.message,
            })
        }
    }

    fn read_frame(&mut self) -> Result<Frame, StreamEnd> {
        let protocol = |message: String| StreamEnd::Failed(ConnectionFailure::Protocol(message));

        let mut line = String::new();
        let read = self
            .reader
            .read_line(&mut line)
            .map_err(|error| StreamEnd::Failed(socket::io_failure(&self.socket_path, error)))?;
        if read == 0 {
            return Err(protocol("the event stream closed".to_owned()));
        }

        let frame: Value =
            serde_json::from_str(&line).map_err(|error| protocol(error.to_string()))?;
        if let Some(error) = frame.get("error") {
            let error = serde_json::from_value(error.clone())
                .map_err(|error| protocol(error.to_string()))?;
            Ok(Frame::Failure(error))
        } else if let Some(result) = frame.get("result") {
            Ok(Frame::Success(result.clone()))
        } else {
            Ok(Frame::Event)
        }
    }
}
