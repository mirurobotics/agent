//! AWS S3, signed with SigV4 from the backend's temporary credentials.

// standard crates
use std::fmt;

// internal crates
use crate::s3::sdk;

// external crates
use aws_credential_types::provider::{error::CredentialsError, future, ProvideCredentials};
use aws_sdk_s3::config::{Credentials as AwsCredentials, IdentityCache, Region, SharedHttpClient};

/// The backend's temporary credentials. Debug only in tests, so they can't
/// reach logs.
#[cfg_attr(test, derive(Clone, Debug))]
pub struct Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: String,
}

/// Where an AWS S3 [`Store`](crate::s3::Store) gets the credentials it signs
/// each request with.
pub trait CredentialSource: Send + Sync + fmt::Debug {
    /// The credentials to sign with now, if there are any.
    fn credentials(&self) -> Option<Credentials>;
}

/// Fixed credentials, for tests.
#[cfg(test)]
impl CredentialSource for Credentials {
    fn credentials(&self) -> Option<Credentials> {
        Some(self.clone())
    }
}

/// The SDK config for an AWS S3 [`Store`](crate::s3::Store). The SDK asks
/// `credentials` before every request (no identity cache), so credentials
/// refreshed mid-upload are used from the next request on.
pub(crate) fn config(
    credentials: impl CredentialSource + 'static,
    region: String,
    http_client: SharedHttpClient,
    force_path_style: bool,
) -> aws_sdk_s3::Config {
    sdk::base_config(http_client)
        .region(Region::new(region))
        .credentials_provider(Provider(credentials))
        .identity_cache(IdentityCache::no_cache())
        .force_path_style(force_path_style)
        .build()
}

/// Hands the SDK the credentials a [`CredentialSource`] has now.
#[derive(Debug)]
struct Provider<T>(T);

impl<T: CredentialSource> ProvideCredentials for Provider<T> {
    fn provide_credentials<'a>(&'a self) -> future::ProvideCredentials<'a>
    where
        Self: 'a,
    {
        let provided = match self.0.credentials() {
            Some(creds) => Ok(AwsCredentials::new(
                creds.access_key_id,
                creds.secret_access_key,
                Some(creds.session_token),
                None,
                "miru-agent",
            )),
            None => Err(CredentialsError::not_loaded("no S3 credentials")),
        };
        future::ProvideCredentials::ready(provided)
    }
}

#[cfg(test)]
mod tests {
    // standard crates
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    // internal crates
    use super::*;
    use crate::s3::store::tests::{obj, req, resp, temp_file_with};
    use crate::s3::{Options, Store};

    // external crates
    use aws_smithy_http_client::test_util::{ReplayEvent, StaticReplayClient};

    /// Credentials whose access key the test can change between requests.
    #[derive(Clone, Debug, Default)]
    struct SwappableKey(Arc<Mutex<String>>);

    impl CredentialSource for SwappableKey {
        fn credentials(&self) -> Option<Credentials> {
            Some(Credentials {
                access_key_id: self.0.lock().unwrap().clone(),
                secret_access_key: "secret".to_string(),
                session_token: "session".to_string(),
            })
        }
    }

    #[derive(Debug)]
    struct NoCredentials;

    impl CredentialSource for NoCredentials {
        fn credentials(&self) -> Option<Credentials> {
            None
        }
    }

    mod config {
        use super::*;

        #[tokio::test]
        async fn asks_for_credentials_before_every_request() {
            let key = SwappableKey::default();
            let put = || ReplayEvent::new(req("PUT", "k?x-id=PutObject"), resp(200, &[]));
            let replay = StaticReplayClient::new(vec![put(), put()]);
            let http_client = SharedHttpClient::new(replay.clone());
            let store = Store::new(Options {
                config: config(key.clone(), "us-east-1".into(), http_client, true),
                gcs: false,
            });
            let src = temp_file_with(b"hello world").await;

            for access_key_id in ["OLD", "NEW"] {
                *key.0.lock().unwrap() = access_key_id.to_string();
                store
                    .put(src.file().clone(), &obj("k"), &HashMap::new())
                    .await
                    .unwrap();
            }

            let auth = replay
                .actual_requests()
                .map(|r| r.headers().get("authorization").unwrap().to_string())
                .collect::<Vec<_>>();
            assert!(auth[0].contains("Credential=OLD/"), "{}", auth[0]);
            assert!(auth[1].contains("Credential=NEW/"), "{}", auth[1]);
        }
    }

    mod provider {
        use super::*;

        #[tokio::test]
        async fn fails_without_credentials() {
            let provided = Provider(NoCredentials).provide_credentials().await;
            assert!(provided.is_err());
        }
    }
}
