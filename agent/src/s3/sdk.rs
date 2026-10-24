//! The aws-sdk-s3 settings every [`Store`](crate::s3::Store) shares, for AWS S3
//! and GCS alike.

// standard crates
use std::time::Duration;

// external crates
use aws_sdk_s3::config::timeout::TimeoutConfig;
use aws_sdk_s3::config::{BehaviorVersion, SharedHttpClient, StalledStreamProtectionConfig};
use aws_smithy_http_client::{proxy, tls, Builder, ConnectorBuilder};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Time limit for bodyless calls, SDK retries and backoff included.
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(60);

/// PutObject/UploadPart get `BODY_ATTEMPT_FLOOR` plus one second per
/// `MIN_UPLOAD_BYTES_PER_SEC` of body: a tail or response stall after the last
/// frame (which stalled-stream protection no longer watches) ends at this limit.
const BODY_ATTEMPT_FLOOR: Duration = Duration::from_secs(90);
const MIN_UPLOAD_BYTES_PER_SEC: u64 = 64 * 1024;

/// Longest a request body may go without progress.
const STALL_GRACE: Duration = Duration::from_secs(60);

/// Cap for `CompleteMultipartUpload`, which can take minutes on large uploads.
const COMPLETE_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(600);

/// Best-effort `AbortMultipartUpload` cleanup.
pub(super) const ABORT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(10);

/// One limit for both the whole operation and each attempt, so SDK retries
/// cannot stack past it.
fn timeout_config(limit: Duration) -> TimeoutConfig {
    TimeoutConfig::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .operation_timeout(limit)
        .operation_attempt_timeout(limit)
        .build()
}

pub(super) fn body_attempt_timeout(len: u64) -> Duration {
    BODY_ATTEMPT_FLOOR + Duration::from_secs(len.div_ceil(MIN_UPLOAD_BYTES_PER_SEC))
}

/// `CompleteMultipartUpload` shares the attempt budget with the calls before
/// it (create or list, then the parts), so it gets the body budget they left.
pub(super) fn complete_attempt_timeout(size: u64, elapsed: Duration) -> Duration {
    body_attempt_timeout(size)
        .saturating_sub(elapsed)
        .clamp(ATTEMPT_TIMEOUT, COMPLETE_ATTEMPT_TIMEOUT)
}

/// Per-operation config override applying `limit` (see [`timeout_config`]).
pub(super) fn attempt_override(limit: Duration) -> aws_sdk_s3::config::Builder {
    aws_sdk_s3::Config::builder().timeout_config(timeout_config(limit))
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

/// The SDK config builder every store starts from: the shared client, the
/// default call limit, and stalled-stream protection on uploads.
pub(super) fn base_config(http_client: SharedHttpClient) -> aws_sdk_s3::config::Builder {
    aws_sdk_s3::Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .http_client(http_client)
        .timeout_config(timeout_config(ATTEMPT_TIMEOUT))
        .stalled_stream_protection(
            StalledStreamProtectionConfig::enabled()
                .upload_enabled(true)
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
    use crate::s3::store::PART_SIZE;
    use crate::s3::{aws, gcs, Config, Credentials};
    use miru_agent::data_uploads::upload::UploaderOptions;

    // external crates
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
        fn aws_and_gcs_configs_carry_limits_and_stall_protection() {
            let cfg = Config {
                creds: Credentials {
                    access_key_id: "AKIA_TEST".to_string(),
                    secret_access_key: "secret".to_string(),
                    session_token: "session".to_string(),
                },
                region: "us-west-2".to_string(),
            };
            let token = gcs::Token::new("gcs-token").unwrap();
            let timeouts = TimeoutConfig::builder()
                .connect_timeout(Duration::from_secs(15))
                .operation_timeout(Duration::from_secs(60))
                .operation_attempt_timeout(Duration::from_secs(60))
                .build();

            for (name, config) in [
                ("aws", aws::config(cfg, http_client(), false)),
                ("gcs", gcs::config(token, http_client())),
            ] {
                assert_eq!(config.timeout_config(), Some(&timeouts), "{name}");
                let ssp = config
                    .stalled_stream_protection()
                    .expect("stall protection configured");
                assert!(ssp.upload_enabled(), "{name}");
                assert_eq!(ssp.grace_period(), Duration::from_secs(60), "{name}");
            }
        }
    }

    mod limits {
        use super::*;

        #[test]
        fn call_limits_stay_under_uploader_backstop() {
            const MIB: u64 = 1024 * 1024;
            let secs = Duration::from_secs;
            assert_eq!(body_attempt_timeout(0), secs(90));
            assert_eq!(body_attempt_timeout(PART_SIZE), secs(218));
            assert_eq!(complete_attempt_timeout(PART_SIZE + 1, secs(0)), secs(219));
            assert_eq!(complete_attempt_timeout(1024 * MIB, secs(0)), secs(600));
            assert_eq!(
                complete_attempt_timeout(PART_SIZE + 1, secs(3600)),
                secs(60)
            );

            let backstop = |len: u64| UploaderOptions::default().attempt_deadline(len);
            // PutObject/UploadPart: the call limit after up to 30 s of backend create.
            for len in [0, MIB, PART_SIZE] {
                assert!(
                    body_attempt_timeout(len) + secs(30) <= backstop(len),
                    "len {len}"
                );
            }
            // Multipart: 20 s backend create, then the S3 create and the parts at the
            // minimum rate eat into complete's budget, and the abort follows. Once
            // create + parts exceed `parts + 30 s`, complete's 60 s floor can
            // overshoot the backstop; that is accepted and not covered here.
            for len in [PART_SIZE + 1, 20 * MIB, 1024 * MIB] {
                let parts = secs(len.div_ceil(64 * 1024));
                for s3_create in [secs(0), secs(30)] {
                    let spent = s3_create + parts;
                    let complete = complete_attempt_timeout(len, spent);
                    let total = secs(20) + spent + complete + ABORT_ATTEMPT_TIMEOUT;
                    assert!(total <= backstop(len), "len {len}, create {s3_create:?}");
                }
            }
        }
    }
}
