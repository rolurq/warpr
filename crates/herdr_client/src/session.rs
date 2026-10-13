use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The session Herdr starts and attaches to when none is named.
pub const DEFAULT_SESSION: &str = "default";

const SOCKET_FILE: &str = "herdr.sock";

/// A Herdr session name, validated to be usable as a directory name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SessionName(String);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("{0:?} is not a valid Herdr session name")]
pub struct InvalidSessionName(String);

impl SessionName {
    pub fn new(name: impl Into<String>) -> Result<Self, InvalidSessionName> {
        let name = name.into();
        let is_single_path_component =
            !matches!(name.as_str(), "" | "." | "..") && !name.contains(['/', '\\', '\0']);
        if is_single_path_component {
            Ok(Self(name))
        } else {
            Err(InvalidSessionName(name))
        }
    }

    pub fn default_session() -> Self {
        Self(DEFAULT_SESSION.to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A Herdr session together with the socket its server listens on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionTarget {
    name: SessionName,
    socket_path: PathBuf,
}

impl SessionTarget {
    /// Locates `name` under Herdr's config directory `config_dir` (see [`config_dir`]).
    pub fn new(config_dir: &Path, name: SessionName) -> Self {
        let socket_path = if name.as_str() == DEFAULT_SESSION {
            config_dir.join(SOCKET_FILE)
        } else {
            config_dir
                .join("sessions")
                .join(name.as_str())
                .join(SOCKET_FILE)
        };
        Self { name, socket_path }
    }

    pub fn name(&self) -> &SessionName {
        &self.name
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }
}

/// Herdr's config directory for the current user, which holds one socket per session.
///
/// Herdr uses `$XDG_CONFIG_HOME/herdr` and falls back to `~/.config/herdr` on every platform,
/// including macOS, so the platform's native config directory is deliberately not used.
pub fn config_dir() -> Option<PathBuf> {
    config_dir_from(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    )
}

pub(crate) fn config_dir_from(xdg_config_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let base = match xdg_config_home.filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(home.filter(|dir| !dir.is_empty())?).join(".config"),
    };
    Some(base.join("herdr"))
}
