// standard crates
use std::sync::Arc;

// internal crates
use crate::crypt::base64;
use crate::server::errors::{GenerateTokenErr, ServerErr};
use crate::trace;

// external crates
use aws_lc_rs::constant_time::verify_slices_are_equal;
use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use secrecy::{ExposeSecret, SecretString};
use tracing::warn;

/// Number of random bytes in a token; base64url without padding encodes them
/// as 43 header-safe characters.
const TOKEN_BYTES: usize = 32;

/// Bearer token that TCP clients of the local device API must present. Debug
/// output redacts the secret.
#[derive(Debug)]
pub struct Token(SecretString);

impl Token {
    /// Generate a token from 32 bytes of the system CSPRNG.
    pub fn generate() -> Result<Token, ServerErr> {
        let mut bytes = [0u8; TOKEN_BYTES];
        aws_lc_rs::rand::fill(&mut bytes)
            .map_err(|_| ServerErr::GenerateTokenErr(GenerateTokenErr { trace: trace!() }))?;
        let encoded = base64::encode_bytes_url_safe_no_pad(&bytes);
        Ok(Token(SecretString::from(encoded)))
    }

    /// The raw token string.
    pub fn expose(&self) -> &str {
        self.0.expose_secret()
    }
}

/// Reject requests without `Authorization: Bearer <token>` with 401 and
/// `WWW-Authenticate: Bearer`. Logs never include the presented credential.
pub async fn check_bearer(token: Arc<Token>, req: Request, next: Next) -> Response {
    if !authorized(&req, &token) {
        warn!("Rejected local device API request without a valid bearer token");
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
        )
            .into_response();
    }
    next.run(req).await
}

/// The scheme matches case-insensitively; the credentials compare in constant
/// time.
fn authorized(req: &Request, token: &Token) -> bool {
    let Some(value) = req.headers().get(header::AUTHORIZATION) else {
        return false;
    };
    let Ok(value) = value.to_str() else {
        return false;
    };
    let Some((scheme, credentials)) = value.split_once(' ') else {
        return false;
    };
    scheme.eq_ignore_ascii_case("Bearer")
        && verify_slices_are_equal(credentials.as_bytes(), token.expose().as_bytes()).is_ok()
}
