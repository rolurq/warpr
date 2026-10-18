use serde::Deserialize;
use warp_graphql::scalars::time::ServerTimestamp;

use super::{AnonymousUserType, FirebaseAuthTokens, PersonalObjectLimits, UserMetadata};
use crate::UserUid;

/// Helper function to set `true` as the default for a serde field on PersistedUser.
fn default_as_true() -> bool {
    true
}

/// The serialized representation of a user, as supplied through the build-time `WARP_USER_SECRET`
/// variable.
#[derive(Debug, Clone, Deserialize)]
pub struct PersistedUser {
    /// Information about the user's authentication through Firebase.
    #[serde(rename = "id_token")]
    pub auth_tokens: FirebaseAuthTokens,
    /// DO NOT USE! This used to be one of the two places we stored the user's Firebase refresh
    /// token. Now, all callers should go through `auth_tokens` to access it.
    #[serde(default)]
    #[deprecated = "use auth_tokens.refresh_token instead"]
    pub refresh_token: String,
    /// The Firebase UID of this user.
    pub local_id: UserUid,
    /// Metadata about the user.
    #[serde(flatten)]
    pub metadata: UserMetadata,
    /// Whether or not the user is onboarded.
    #[serde(default = "default_as_true")]
    pub is_onboarded: bool,
    /// Whether or not the user needs to link their account via SSO due to an organization setting.
    #[serde(default)]
    pub needs_sso_link: bool,
    /// What type of anonymous user this user is. May be `None` if they are not anonymous.
    #[serde(default)]
    pub anonymous_user_type: Option<AnonymousUserType>,
    #[serde(default)]
    pub linked_at: Option<ServerTimestamp>,
    #[serde(default)]
    pub personal_object_limits: Option<PersonalObjectLimits>,
    /// Whether or not this user is on what we consider a "work" domain.
    #[serde(default)]
    pub is_on_work_domain: bool,
}

#[cfg(test)]
#[path = "persistence_tests.rs"]
mod tests;
