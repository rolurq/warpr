//! The display-only metadata tokens Herdr shows beside a workspace in its sidebar.

use std::collections::BTreeMap;

/// A named value an integration reported for a workspace, such as a version-control status.
///
/// Herdr drops a token from its snapshots when the token is cleared or its time-to-live runs out,
/// so a token that is presented is one Herdr still shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetadataToken {
    pub name: String,
    pub value: String,
}

/// Herdr reports tokens as a JSON object without a meaningful order, so they are presented sorted
/// by name to keep the display stable between reads.
pub(crate) fn presented_tokens(reported: BTreeMap<String, String>) -> Vec<MetadataToken> {
    reported
        .into_iter()
        .map(|(name, value)| MetadataToken { name, value })
        .collect()
}

#[cfg(all(test, unix))]
#[path = "metadata_tests.rs"]
mod tests;
