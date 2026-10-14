pub mod auth;
pub mod base_client;
pub mod drive;
pub mod graphql_helpers;
pub mod iap;
pub mod ids;
pub mod network_logging;
mod public_api;

pub use auth::UserUid;
pub use cloud_objects::server_id_traits;
pub use public_api::HttpStatusError;

/// Returns an error instead of letting a request reach Warp's hosted agent backend while the
/// `DisableHostedAgent` kill switch is on. Call before opening any hosted-agent connection.
pub fn ensure_hosted_agent_enabled() -> anyhow::Result<()> {
    anyhow::ensure!(
        !warp_core::features::FeatureFlag::DisableHostedAgent.is_enabled(),
        "Warp's hosted agent is disabled"
    );
    Ok(())
}
