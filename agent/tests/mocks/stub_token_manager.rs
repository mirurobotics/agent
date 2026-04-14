// standard crates
use std::sync::{Arc, Mutex};

// internal crates
use miru_agent::authn::{AuthnErr, Token, TokenManagerExt};

// external crates
use chrono::Utc;

/// One-shot test stub: each call to `get_token` consumes the canned
/// response. `http::with_retry` does not re-fetch the token between
/// attempts, so a single canned response is sufficient for every test
/// that currently exercises `HttpBackend`. Calling `get_token` more
/// than once panics with "no canned response".
pub struct StubTokenManager {
    response: Mutex<Option<Result<Arc<Token>, AuthnErr>>>,
}

impl StubTokenManager {
    pub fn ok(token: &str) -> Self {
        let t = Token {
            token: token.to_string(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
        };
        Self {
            response: Mutex::new(Some(Ok(Arc::new(t)))),
        }
    }
    pub fn err(e: AuthnErr) -> Self {
        Self {
            response: Mutex::new(Some(Err(e))),
        }
    }
}

impl TokenManagerExt for StubTokenManager {
    async fn shutdown(&self) -> Result<(), AuthnErr> {
        Ok(())
    }

    async fn get_token(&self) -> Result<Arc<Token>, AuthnErr> {
        self.response
            .lock()
            .unwrap()
            .take()
            .expect("StubTokenManager: no canned token response")
    }

    async fn refresh_token(&self) -> Result<(), AuthnErr> {
        Ok(())
    }
}
