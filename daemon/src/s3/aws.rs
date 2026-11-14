//! AWS S3, signed with SigV4 from the backend's temporary credentials.

// internal crates
use crate::s3::sdk;

// external crates
use aws_sdk_s3::config::{Credentials as AwsCredentials, Region, SharedHttpClient};

pub struct Config {
    pub creds: Credentials,
    pub region: String,
}

pub struct Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: String,
}

/// The SDK config for an AWS S3 [`Store`](crate::s3::Store).
pub fn config(
    cfg: Config,
    http_client: SharedHttpClient,
    force_path_style: bool,
) -> aws_sdk_s3::Config {
    let s3creds = AwsCredentials::new(
        cfg.creds.access_key_id,
        cfg.creds.secret_access_key,
        Some(cfg.creds.session_token),
        None,
        "miru-agent",
    );
    sdk::base_config(http_client)
        .region(Region::new(cfg.region))
        .credentials_provider(s3creds)
        .force_path_style(force_path_style)
        .build()
}
