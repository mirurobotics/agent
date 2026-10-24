//! The aws-sdk-s3 settings every [`Store`](crate::s3::Store) shares, for AWS S3
//! and GCS alike.

// external crates
use aws_sdk_s3::config::{BehaviorVersion, SharedHttpClient};
use aws_smithy_http_client::{proxy, tls, Builder, ConnectorBuilder};

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
    aws_sdk_s3::Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .http_client(http_client)
}

#[cfg(test)]
mod tests {
    // standard crates
    use std::time::Duration;

    // internal crates
    use super::*;

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
}
