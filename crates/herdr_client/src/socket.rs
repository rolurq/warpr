use std::path::Path;

use serde_json::Value;

use crate::ConnectionFailure;

#[cfg(unix)]
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[cfg(unix)]
pub(crate) fn io_failure(socket_path: &Path, error: std::io::Error) -> ConnectionFailure {
    ConnectionFailure::SocketUnreachable {
        path: socket_path.to_owned(),
        message: error.to_string(),
    }
}

/// Opens a connection to the session socket with read and write timeouts set.
#[cfg(unix)]
pub(crate) fn connect(
    socket_path: &Path,
) -> Result<std::os::unix::net::UnixStream, ConnectionFailure> {
    let unreachable = |error| io_failure(socket_path, error);
    let stream = std::os::unix::net::UnixStream::connect(socket_path).map_err(unreachable)?;
    stream
        .set_read_timeout(Some(TIMEOUT))
        .map_err(unreachable)?;
    stream
        .set_write_timeout(Some(TIMEOUT))
        .map_err(unreachable)?;
    Ok(stream)
}

/// Sends one request over a fresh connection and returns the response's `result`.
///
/// Herdr frames requests and responses as one JSON object per line.
#[cfg(unix)]
pub(crate) fn request(
    socket_path: &Path,
    method: &str,
    params: Value,
) -> Result<Value, ConnectionFailure> {
    use std::io::{BufRead, BufReader, Write};

    use serde::Deserialize;
    use serde_json::json;

    const REQUEST_ID: &str = "warpr";

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Response {
        Success { result: Value },
        Failure { error: ErrorBody },
    }

    #[derive(Deserialize)]
    struct ErrorBody {
        code: String,
        message: String,
    }

    let unreachable = |error| io_failure(socket_path, error);

    let mut stream = connect(socket_path)?;

    let request = json!({ "id": REQUEST_ID, "method": method, "params": params });
    writeln!(stream, "{request}").map_err(unreachable)?;

    let mut line = String::new();
    BufReader::new(&stream)
        .read_line(&mut line)
        .map_err(unreachable)?;
    if line.is_empty() {
        return Err(ConnectionFailure::Protocol(
            "the connection closed before a response arrived".to_owned(),
        ));
    }

    match serde_json::from_str(&line)
        .map_err(|error| ConnectionFailure::Protocol(error.to_string()))?
    {
        Response::Success { result } => Ok(result),
        Response::Failure { error } => Err(ConnectionFailure::Rejected {
            code: error.code,
            message: error.message,
        }),
    }
}

#[cfg(not(unix))]
pub(crate) fn request(
    _socket_path: &Path,
    _method: &str,
    _params: Value,
) -> Result<Value, ConnectionFailure> {
    Err(ConnectionFailure::UnsupportedPlatform)
}
