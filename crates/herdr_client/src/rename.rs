use serde_json::{Value, json};

use crate::{ConnectionFailure, HerdrTabId, SessionTarget, socket};

/// What a rename request names. Each variant is one kind of Herdr thing that has a label, and
/// maps to the Herdr request that renames it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenameTarget {
    Tab(HerdrTabId),
}

impl RenameTarget {
    fn method(&self) -> &'static str {
        match self {
            Self::Tab(_) => "tab.rename",
        }
    }

    fn params(&self, label: &str) -> Value {
        match self {
            Self::Tab(tab) => json!({ "tab_id": tab.as_str(), "label": label }),
        }
    }
}

/// Asks the session of `target` to give `what` the label `label`.
///
/// Nothing is returned to present: the new label reaches a front-end through the session's own
/// state, like any other change Herdr makes. A refused request is
/// [`ConnectionFailure::Rejected`].
pub fn rename(
    target: &SessionTarget,
    what: &RenameTarget,
    label: &str,
) -> Result<(), ConnectionFailure> {
    socket::request(target.socket_path(), what.method(), what.params(label)).map(drop)
}

#[cfg(test)]
#[path = "rename_tests.rs"]
mod tests;
