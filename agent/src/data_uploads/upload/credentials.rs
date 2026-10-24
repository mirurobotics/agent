//! Upload credentials kept valid for the whole transfer.
//!
//! The backend's upload credentials expire, and a large upload can outlast
//! them. [`UploadCredentialSource`] re-vends them in the background
//! (`POST /uploads/{id}/credentials`) [`REFRESH_WINDOW`] before they expire,
//! and the S3 and GCS stores read the current ones on every request.

// standard crates
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::authn::TokenManagerExt;
use crate::clock::Clock;
use crate::data_uploads::upload::errors::{classified_executor_err, UploadErr};
use crate::http::{self, ClientI};
use crate::s3::{self, aws::CredentialSource, gcs::TokenSource};
use backend_api::models::upload_credentials::Scheme;
use backend_api::models::UploadCredentials;

// external crates
use chrono::DateTime;
use tokio::sync::watch;
use tracing::{info, warn};

/// How long before the credentials expire they are re-vended.
const REFRESH_WINDOW: Duration = Duration::from_secs(5 * 60);

/// The wait before retrying a refresh that failed, or that returned
/// credentials already due for one.
const RETRY_DELAY: Duration = Duration::from_secs(30);

/// One upload's current credentials. Clones share them, and the refresh task
/// stops once every clone is dropped.
#[derive(Clone)]
pub struct UploadCredentialSource {
    current: watch::Receiver<UploadCredentials>,
}

impl UploadCredentialSource {
    /// Starts from `creds` and spawns the task that re-vends them for
    /// `upload_id` before they expire, by `clock`'s reading of the server's time.
    pub fn spawn<C: ClientI + 'static, T: TokenManagerExt + 'static>(
        http_client: Arc<C>,
        token_mngr: Arc<T>,
        clock: Clock,
        upload_id: String,
        creds: UploadCredentials,
    ) -> Self {
        let (tx, current) = watch::channel(creds);
        tokio::spawn(refresh(http_client, token_mngr, clock, upload_id, tx));
        Self { current }
    }

    /// Credentials that are never refreshed.
    #[cfg(test)]
    pub(crate) fn fixed(creds: UploadCredentials) -> Self {
        Self {
            current: watch::channel(creds).1,
        }
    }

    /// The credentials to use now.
    pub fn current(&self) -> UploadCredentials {
        self.current.borrow().clone()
    }
}

/// Leaves the credentials out, so they can't reach logs.
impl fmt::Debug for UploadCredentialSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UploadCredentialSource")
    }
}

/// The S3 store signs every request with the current credentials.
impl CredentialSource for UploadCredentialSource {
    fn credentials(&self) -> Option<s3::aws::Credentials> {
        let current = self.current.borrow();
        let creds = current.s3_credentials.as_deref()?;
        Some(s3::aws::Credentials {
            access_key_id: creds.access_key_id.clone(),
            secret_access_key: creds.secret_access_key.clone(),
            session_token: creds.session_token.clone(),
        })
    }
}

/// The GCS store sends the current token on every request.
impl TokenSource for UploadCredentialSource {
    fn token(&self) -> Option<s3::gcs::Token> {
        gcs_token(&self.current.borrow())
    }
}

/// Re-vends the credentials in `tx` before they expire, until every source
/// reading them is dropped. A failed refresh keeps the current credentials and
/// is retried after [`RETRY_DELAY`].
async fn refresh<C: ClientI, T: TokenManagerExt>(
    http_client: Arc<C>,
    token_mngr: Arc<T>,
    clock: Clock,
    upload_id: String,
    tx: watch::Sender<UploadCredentials>,
) {
    let (scheme, mut wait) = {
        let creds = tx.borrow();
        (creds.scheme, until_refresh(&creds, &clock))
    };
    loop {
        tokio::select! {
            biased;
            _ = tx.closed() => return,
            _ = tokio::time::sleep(wait) => {}
        }
        wait = match vend(http_client.as_ref(), token_mngr.as_ref(), &upload_id).await {
            Ok(creds) if usable_for(&creds, scheme) => {
                info!("upload: refreshed the credentials of upload {upload_id}");
                let wait = until_refresh(&creds, &clock);
                tx.send_replace(creds);
                wait.max(RETRY_DELAY)
            }
            Ok(_) => {
                warn!("upload: new credentials don't fit upload {upload_id} ({scheme:?})");
                RETRY_DELAY
            }
            Err(e) => {
                warn!("upload: refreshing the credentials of upload {upload_id} failed: {e}");
                RETRY_DELAY
            }
        };
    }
}

/// Vends new credentials for `upload_id`.
async fn vend<C: ClientI, T: TokenManagerExt>(
    http_client: &C,
    token_mngr: &T,
    upload_id: &str,
) -> Result<UploadCredentials, UploadErr> {
    let token = token_mngr
        .get_token()
        .await
        .map_err(classified_executor_err)?;
    let params = http::uploads::VendCredentialsParams {
        id: upload_id,
        token: &token.token,
    };
    http::uploads::vend_credentials(http_client, params)
        .await
        .map_err(classified_executor_err)
}

/// How long until `creds` are due for a refresh, [`REFRESH_WINDOW`] before
/// they expire by the server's clock (the robot's own clock may be years off).
/// Zero once that has passed, or if the expiry can't be read or the server's
/// time isn't known yet.
fn until_refresh(creds: &UploadCredentials, clock: &Clock) -> Duration {
    let Some(now) = clock.server_now() else {
        return Duration::ZERO;
    };
    DateTime::parse_from_rfc3339(&creds.expires_at)
        .ok()
        .and_then(|expires_at| expires_at.signed_duration_since(now).to_std().ok())
        .map_or(Duration::ZERO, |left| left.saturating_sub(REFRESH_WINDOW))
}

/// Whether `creds` can replace credentials of `scheme`: the same scheme, with
/// credentials its store can send.
fn usable_for(creds: &UploadCredentials, scheme: Scheme) -> bool {
    let sendable = match scheme {
        Scheme::S3 => creds.s3_credentials.is_some(),
        Scheme::Gcs => gcs_token(creds).is_some(),
        Scheme::SchemeUnknown => false,
    };
    creds.scheme == scheme && sendable
}

/// The GCS token in `creds`, if there is one that can be sent.
fn gcs_token(creds: &UploadCredentials) -> Option<s3::gcs::Token> {
    s3::gcs::Token::new(creds.gcs_credentials.as_deref()?.access_token.clone())
}

#[cfg(test)]
mod tests {
    // standard crates
    use std::sync::atomic::{AtomicUsize, Ordering};

    // internal crates
    use super::*;
    use crate::test_utils::{
        http_client::{Call, MockClient},
        upload::token_manager,
    };
    use miru_agent::http::errors::{HTTPErr, MockErr};

    // external crates
    use chrono::Utc;
    use serde_json::json;

    fn s3_creds(access_key_id: &str, expires_at: &str) -> UploadCredentials {
        serde_json::from_value(json!({
            "scheme": "s3",
            "s3_credentials": {
                "scheme": "s3",
                "access_key_id": access_key_id,
                "secret_access_key": "secret",
                "session_token": "session",
                "region": "us-east-1",
                "expires_at": expires_at
            },
            "gcs_credentials": null,
            "expires_at": expires_at
        }))
        .unwrap()
    }

    fn gcs_creds(access_token: &str, expires_at: &str) -> UploadCredentials {
        serde_json::from_value(json!({
            "scheme": "gcs",
            "s3_credentials": null,
            "gcs_credentials": {
                "scheme": "gcs",
                "access_token": access_token,
                "expires_at": expires_at
            },
            "expires_at": expires_at
        }))
        .unwrap()
    }

    /// The server's time in these tests, years from the robot's own clock, so
    /// only a refresh timed by the server's clock passes.
    fn server_time() -> DateTime<Utc> {
        "2030-01-01T00:00:00Z".parse().unwrap()
    }

    /// A clock that has heard [`server_time`] from the backend.
    fn clock() -> Clock {
        let clock = Clock::new();
        clock.observe_server_time(server_time());
        clock
    }

    fn in_an_hour() -> String {
        (server_time() + chrono::Duration::hours(1)).to_rfc3339()
    }

    fn access_key_id(source: &UploadCredentialSource) -> String {
        source.current().s3_credentials.unwrap().access_key_id
    }

    fn spawn_source(client: &Arc<MockClient>, creds: UploadCredentials) -> UploadCredentialSource {
        UploadCredentialSource::spawn(
            client.clone(),
            token_manager(),
            clock(),
            "upl_1".into(),
            creds,
        )
    }

    fn vends(client: &MockClient) -> usize {
        client.call_count(Call::VendUploadCredentials)
    }

    mod refresh {
        use super::*;

        #[tokio::test(start_paused = true)]
        async fn refreshes_before_expiry() {
            let client = Arc::new(MockClient::default());
            client.set_vend_upload_credentials(|| Ok(s3_creds("NEW", &in_an_hour())));
            let source = spawn_source(&client, s3_creds("OLD", &in_an_hour()));

            tokio::time::sleep(Duration::from_secs(54 * 60)).await;
            assert_eq!(access_key_id(&source), "OLD");
            assert_eq!(vends(&client), 0);

            tokio::time::sleep(Duration::from_secs(2 * 60)).await;
            assert_eq!(access_key_id(&source), "NEW");
            let requests = client.requests();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].path, "/uploads/upl_1/credentials");
        }

        #[tokio::test(start_paused = true)]
        async fn keeps_the_current_credentials_until_a_refresh_succeeds() {
            // A failed vend, then credentials for the wrong scheme, then good ones.
            let client = Arc::new(MockClient::default());
            let attempts = AtomicUsize::new(0);
            client.set_vend_upload_credentials(move || {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                match attempt {
                    0 => Err(HTTPErr::MockErr(MockErr {
                        is_network_conn_err: true,
                    })),
                    1 => Ok(gcs_creds("gcs-token", &in_an_hour())),
                    _ => Ok(s3_creds("NEW", &in_an_hour())),
                }
            });
            // Already expired, so the first refresh is due at once.
            let source = spawn_source(&client, s3_creds("OLD", "2021-01-01T00:00:00Z"));

            let start = tokio::time::Instant::now();
            for (secs, vended, key) in [(1, 1, "OLD"), (31, 2, "OLD"), (61, 3, "NEW")] {
                tokio::time::sleep_until(start + Duration::from_secs(secs)).await;
                assert_eq!(vends(&client), vended, "after {secs}s");
                assert_eq!(access_key_id(&source), key, "after {secs}s");
            }
        }

        #[tokio::test(start_paused = true)]
        async fn stops_once_every_source_is_dropped() {
            let client = Arc::new(MockClient::default());
            client.set_vend_upload_credentials(|| Ok(s3_creds("NEW", &in_an_hour())));
            let source = spawn_source(&client, s3_creds("OLD", &in_an_hour()));
            let clone = source.clone();
            drop(source);

            // The clone keeps the refresh going.
            tokio::time::sleep(Duration::from_secs(60 * 60)).await;
            assert_eq!(vends(&client), 1);

            drop(clone);
            tokio::time::sleep(Duration::from_secs(2 * 60 * 60)).await;
            assert_eq!(vends(&client), 1);
        }

        #[test]
        fn refresh_is_due_before_expiry() {
            let due = until_refresh(&s3_creds("OLD", &in_an_hour()), &clock());
            assert!((54 * 60..=55 * 60).contains(&due.as_secs()), "{due:?}");

            for expires_at in ["2021-01-01T00:00:00Z", "not a time"] {
                let due = until_refresh(&s3_creds("OLD", expires_at), &clock());
                assert_eq!(due, Duration::ZERO, "{expires_at}");
            }
        }

        #[test]
        fn refresh_is_due_at_once_until_the_servers_time_is_known() {
            let due = until_refresh(&s3_creds("OLD", &in_an_hour()), &Clock::new());
            assert_eq!(due, Duration::ZERO);
        }

        #[test]
        fn only_credentials_the_store_can_send_replace_the_current_ones() {
            let s3 = s3_creds("NEW", "2099-01-01T00:00:00Z");
            let gcs = gcs_creds("new", "2099-01-01T00:00:00Z");
            let unsendable = gcs_creds("bad\ntoken", "2099-01-01T00:00:00Z");
            let empty = UploadCredentials::default();
            let unknown = UploadCredentials {
                scheme: Scheme::SchemeUnknown,
                ..Default::default()
            };

            assert!(usable_for(&s3, Scheme::S3));
            assert!(usable_for(&gcs, Scheme::Gcs));
            // Another scheme, no credentials, or a token that can't be sent.
            assert!(!usable_for(&gcs, Scheme::S3));
            assert!(!usable_for(&empty, Scheme::S3));
            assert!(!usable_for(&unsendable, Scheme::Gcs));
            assert!(!usable_for(&unknown, Scheme::SchemeUnknown));
        }
    }

    mod stores {
        use super::*;

        #[test]
        fn s3_store_gets_the_s3_credentials() {
            let source = UploadCredentialSource::fixed(s3_creds("AKIA_TEST", &in_an_hour()));
            let creds = source.credentials().unwrap();
            assert_eq!(
                (
                    creds.access_key_id.as_str(),
                    creds.secret_access_key.as_str(),
                    creds.session_token.as_str(),
                ),
                ("AKIA_TEST", "secret", "session")
            );
        }

        #[test]
        fn each_store_finds_nothing_in_the_other_schemes_credentials() {
            let gcs = UploadCredentialSource::fixed(gcs_creds("token", &in_an_hour()));
            assert!(gcs.credentials().is_none());
            let s3 = UploadCredentialSource::fixed(s3_creds("OLD", &in_an_hour()));
            assert!(s3.token().is_none());
        }
    }

    mod debug {
        use super::*;

        #[test]
        fn leaves_the_credentials_out() {
            let source = UploadCredentialSource::fixed(s3_creds("AKIA_SECRET", &in_an_hour()));
            assert_eq!(format!("{source:?}"), "UploadCredentialSource");
        }
    }
}
