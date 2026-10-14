use std::time::Duration;

/// How a [`crate::LiveSession`] retries after losing its connection to Herdr.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReconnectPolicy {
    /// Wait before the first retry. It doubles with every further consecutive failure.
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    /// Consecutive failed attempts after which the session reports
    /// [`crate::ConnectionStatus::Disconnected`] instead of
    /// [`crate::ConnectionStatus::Reconnecting`]. Retrying goes on regardless, at `max_backoff`.
    pub disconnected_after: u32,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            initial_backoff: Duration::from_millis(250),
            max_backoff: Duration::from_secs(5),
            disconnected_after: 20,
        }
    }
}

impl ReconnectPolicy {
    /// How long to wait after `failed_attempts` consecutive failures.
    pub(crate) fn backoff(&self, failed_attempts: u32) -> Duration {
        let doublings = failed_attempts.saturating_sub(1).min(31);
        self.initial_backoff
            .saturating_mul(1 << doublings)
            .min(self.max_backoff)
    }
}
