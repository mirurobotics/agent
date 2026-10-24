//! The aws-sdk-s3 settings every [`Store`](crate::s3::Store) shares, for AWS S3
//! and GCS alike.

// standard crates
use std::time::Duration;

// external crates
use aws_sdk_s3::config::timeout::TimeoutConfig;
use aws_sdk_s3::config::{BehaviorVersion, SharedHttpClient, StalledStreamProtectionConfig};
use aws_smithy_http_client::{proxy, tls, Builder, ConnectorBuilder};

/// Longer than the SDK default, for slow links.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Longest a request or response body may go without moving a byte.
const STALL_GRACE: Duration = Duration::from_secs(60);

/// Time limit for a call, SDK retries and backoff included, unless the call
/// sets its own.
const CALL_TIMEOUT: Duration = Duration::from_secs(60);

/// Completing a large multipart upload can take minutes.
pub(super) const COMPLETE_TIMEOUT: Duration = Duration::from_secs(600);

/// The abort is best-effort cleanup, so it gives up early.
pub(super) const ABORT_TIMEOUT: Duration = Duration::from_secs(10);

/// An upload call has time to finish at this rate or faster.
const MIN_UPLOAD_BYTES_PER_SEC: u64 = 64 * 1024;

/// Time limit for a call that uploads `len` bytes. Stall protection stops
/// watching once the last frame is sent, so this is what ends a call whose
/// response never comes.
pub(super) fn upload_timeout(len: u64) -> Duration {
    CALL_TIMEOUT + Duration::from_secs(len.div_ceil(MIN_UPLOAD_BYTES_PER_SEC))
}

/// Config override that sets one call's time limit. The SDK lays it over the
/// client's config, so all other settings, the connect timeout included, stay.
pub(super) fn timeout(limit: Duration) -> aws_sdk_s3::config::Builder {
    let timeouts = TimeoutConfig::builder().operation_timeout(limit).build();
    aws_sdk_s3::Config::builder().timeout_config(timeouts)
}

/// One HTTPS client for every store built from it, so uploads reuse pooled
/// connections instead of a fresh TCP+TLS handshake per upload. Mirrors the
/// SDK default client: rustls with aws-lc, proxy settings from the environment.
pub fn http_client() -> SharedHttpClient {
    Builder::new().build_with_connector_fn(|settings, components| {
        let provider = tls::Provider::Rustls(tls::rustls_provider::CryptoMode::AwsLc);
        let mut conn = ConnectorBuilder::default().tls_provider(provider);
        conn.set_connector_settings(settings.cloned());
        if let Some(components) = components {
            conn.set_sleep_impl(components.sleep_impl());
        }
        conn.set_proxy_config(Some(proxy::ProxyConfig::from_env()));
        conn.build()
    })
}

/// The SDK config builder every store starts from.
pub(super) fn base_config(http_client: SharedHttpClient) -> aws_sdk_s3::config::Builder {
    let timeouts = TimeoutConfig::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .operation_timeout(CALL_TIMEOUT)
        .build();
    aws_sdk_s3::Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .http_client(http_client)
        .timeout_config(timeouts)
        .stalled_stream_protection(
            StalledStreamProtectionConfig::enabled()
                .grace_period(STALL_GRACE)
                .build(),
        )
}

#[cfg(test)]
mod tests {
    // standard crates
    use std::time::Duration;

    // internal crates
    use super::*;
    use crate::s3::{aws, gcs};

    // external crates
    use aws_sdk_s3::config::Credentials;
    use aws_smithy_runtime_api::client::http::{HttpClient, HttpConnectorSettings};
    use aws_smithy_runtime_api::client::runtime_components::RuntimeComponentsBuilder;

    mod http_client {
        use super::*;

        #[tokio::test]
        async fn http_client_builds_a_connector() {
            // The SDK calls the connector closure only on a request, so call it
            // directly. The client times the call, so it needs a real time source.
            let time_source = aws_sdk_s3::Config::builder().build().time_source();
            let components = RuntimeComponentsBuilder::for_tests()
                .with_time_source(time_source)
                .build()
                .unwrap();
            let settings = HttpConnectorSettings::builder()
                .connect_timeout(Duration::from_secs(15))
                .build();
            let _connector = http_client().http_connector(&settings, &components);
        }
    }

    mod base_config {
        use super::*;

        #[test]
        fn aws_and_gcs_configs_carry_timeouts_and_stall_protection() {
            let creds = Credentials::new("AKIA_TEST", "secret", None, None, "test");
            let aws_config = aws::config(creds, "us-west-2".into(), http_client(), false);
            let token = gcs::Token::new("gcs-token").unwrap();
            let gcs_config = gcs::config(token, http_client());
            let timeouts = TimeoutConfig::builder()
                .connect_timeout(Duration::from_secs(15))
                .operation_timeout(Duration::from_secs(60))
                .build();

            for (name, config) in [("aws", aws_config), ("gcs", gcs_config)] {
                assert_eq!(config.timeout_config(), Some(&timeouts), "{name}");
                let ssp = config
                    .stalled_stream_protection()
                    .expect("stall protection configured");
                assert!(ssp.upload_enabled(), "{name}");
                assert_eq!(ssp.grace_period(), Duration::from_secs(60), "{name}");
            }
        }
    }
}
