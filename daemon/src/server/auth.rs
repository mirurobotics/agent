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
pub struct BearerToken(SecretString);

impl BearerToken {
    /// Generate a token from 32 bytes of the system CSPRNG.
    pub fn generate() -> Result<BearerToken, ServerErr> {
        let mut bytes = [0u8; TOKEN_BYTES];
        aws_lc_rs::rand::fill(&mut bytes)
            .map_err(|_| ServerErr::GenerateTokenErr(GenerateTokenErr { trace: trace!() }))?;
        let encoded = base64::encode_bytes_url_safe_no_pad(&bytes);
        Ok(BearerToken(SecretString::from(encoded)))
    }

    /// The raw token string.
    pub fn expose(&self) -> &str {
        self.0.expose_secret()
    }

    /// The token still wrapped, for code that stores it without reading it.
    pub fn secret(&self) -> &SecretString {
        &self.0
    }
}

/// Reject requests without `Authorization: Bearer <token>` with 401 and
/// `WWW-Authenticate: Bearer`. Logs never include the presented credential.
pub async fn check_bearer(token: Arc<BearerToken>, req: Request, next: Next) -> Response {
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
fn authorized(req: &Request, token: &BearerToken) -> bool {
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

#[cfg(test)]
mod tests {
    // internal crates
    use super::*;

    // external crates
    use axum::body::Body;
    use axum::http::HeaderValue;
    use axum::middleware::from_fn;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    async fn call(token: Arc<BearerToken>, auth: Option<HeaderValue>) -> Response {
        let app = Router::new()
            .route("/health", get(|| async { StatusCode::OK }))
            .layer(from_fn(move |req, next| {
                check_bearer(token.clone(), req, next)
            }));
        let mut builder = Request::builder().uri("/health");
        if let Some(auth) = auth {
            builder = builder.header(header::AUTHORIZATION, auth);
        }
        app.oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    async fn status_for(token: Arc<BearerToken>, auth: &str) -> StatusCode {
        let auth = HeaderValue::from_str(auth).unwrap();
        call(token, Some(auth)).await.status()
    }

    fn token() -> Arc<BearerToken> {
        Arc::new(BearerToken::generate().unwrap())
    }

    #[test]
    fn generate_yields_43_char_base64url() {
        let token = BearerToken::generate().unwrap();
        let raw = token.expose();
        assert_eq!(raw.len(), 43);
        assert!(
            raw.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "token has a non-base64url character: {raw}"
        );
    }

    #[test]
    fn generate_is_unique() {
        let a = BearerToken::generate().unwrap();
        let b = BearerToken::generate().unwrap();
        assert_ne!(a.expose(), b.expose());
    }

    #[test]
    fn debug_redacts_secret() {
        let token = BearerToken::generate().unwrap();
        let debug = format!("{token:?}");
        assert!(
            !debug.contains(token.expose()),
            "debug leaks token: {debug}"
        );
    }

    #[tokio::test]
    async fn missing_header_is_401() {
        let response = call(token(), None).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response.headers().get(header::WWW_AUTHENTICATE).unwrap(),
            "Bearer"
        );
    }

    #[tokio::test]
    async fn basic_scheme_is_401() {
        let token = token();
        let auth = format!("Basic {}", token.expose());
        assert_eq!(status_for(token, &auth).await, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn wrong_token_is_401() {
        let other = BearerToken::generate().unwrap();
        let auth = format!("Bearer {}", other.expose());
        assert_eq!(status_for(token(), &auth).await, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn different_length_token_is_401() {
        let token = token();
        let raw = token.expose();
        let auth = format!("Bearer {}", &raw[..raw.len() - 1]);
        assert_eq!(status_for(token, &auth).await, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn empty_credentials_is_401() {
        assert_eq!(
            status_for(token(), "Bearer ").await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn token_without_scheme_is_401() {
        let token = token();
        let auth = token.expose().to_string();
        assert_eq!(status_for(token, &auth).await, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn non_ascii_header_is_401() {
        let auth = HeaderValue::from_bytes(b"Bearer \xff").unwrap();
        let response = call(token(), Some(auth)).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn correct_token_is_200() {
        let token = token();
        let auth = format!("Bearer {}", token.expose());
        assert_eq!(status_for(token, &auth).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn lowercase_scheme_is_200() {
        let token = token();
        let auth = format!("bearer {}", token.expose());
        assert_eq!(status_for(token, &auth).await, StatusCode::OK);
    }
}
