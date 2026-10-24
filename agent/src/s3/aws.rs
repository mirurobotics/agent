//! AWS S3, signed with SigV4 from the backend's temporary credentials.

// internal crates
use crate::s3::sdk;

// external crates
use aws_sdk_s3::config::{IdentityCache, ProvideCredentials, Region, SharedHttpClient};

/// The SDK config for an AWS S3 [`Store`](crate::s3::Store). The SDK asks
/// `credentials` before every request (no identity cache), so credentials
/// refreshed mid-upload are used from the next request on.
pub(crate) fn config(
    credentials: impl ProvideCredentials + 'static,
    region: String,
    http_client: SharedHttpClient,
    force_path_style: bool,
) -> aws_sdk_s3::Config {
    sdk::base_config(http_client)
        .region(Region::new(region))
        .credentials_provider(credentials)
        .identity_cache(IdentityCache::no_cache())
        .force_path_style(force_path_style)
        .build()
}
