// internal crates
use crate::authn::{
    errors::{AuthnErr, SerdeErr, TimestampConversionErr},
    token::Token,
};
use crate::crypt::{base64, rsa};
use crate::filesys::file::File;
use crate::http::{self, devices};
use crate::trace;

// external crates
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use uuid::Uuid;

#[derive(Serialize)]
struct JwtHeader {
    alg: &'static str,
    typ: &'static str,
    jwk: rsa::Jwk,
}

#[derive(Serialize)]
struct JwtPayload {
    jti: String,
    iat: i64,
    exp: i64,
}

/// Serialize `value` to JSON bytes, then base64url-no-pad-encode the result.
/// `serde_json::to_vec` cannot fail for the structs we feed it (no custom
/// serializers, no recursive references), but we still propagate the error
/// for defense in depth.
fn jwt_segment<T: Serialize>(value: &T) -> Result<String, AuthnErr> {
    let bytes = serde_json::to_vec(value).map_err(|e| {
        AuthnErr::SerdeErr(SerdeErr {
            source: e,
            trace: trace!(),
        })
    })?;
    Ok(base64::encode_bytes_url_safe_no_pad(&bytes))
}

/// Mint a fresh device token by building a self-signed RS512 JWT carrying
/// the device's public key as a JWK header (RFC 7517) and posting it to
/// the backend's `/devices/issue_token` endpoint.
///
/// The server identifies the device by the SHA-256 fingerprint of the JWK
/// in the header, so no `device_id` is required on the wire.
///
/// This free function is the shared core used by both
/// `SingleThreadTokenManager::issue_token` (the long-running token refresh
/// path inside the agent) and `app::upgrade::reconcile` (the boot-time
/// rebootstrap path that does not yet have a `TokenManager` set up).
pub async fn issue_token<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    private_key_file: &File,
    public_key_file: &File,
) -> Result<Token, AuthnErr> {
    // build the self-signed JWT
    let jwt = build_self_signed_jwt(private_key_file, public_key_file).await?;

    // send the token request
    let resp = devices::issue_token(http_client, devices::IssueTokenParams { token: &jwt }).await?;

    // format the response
    let expires_at = resp.expires_at.parse::<DateTime<Utc>>().map_err(|e| {
        AuthnErr::TimestampConversionErr(TimestampConversionErr {
            msg: format!(
                "failed to parse date time '{}' from string: {}",
                resp.expires_at, e
            ),
            trace: trace!(),
        })
    })?;
    Ok(Token {
        token: resp.token,
        expires_at,
    })
}

/// Build a self-signed RS512 JWT (RFC 7519) whose header carries the
/// device's public key as a JWK (RFC 7517). The payload contains a unique
/// `jti`, the current `iat`, and an `exp` two minutes in the future.
async fn build_self_signed_jwt(
    private_key_file: &File,
    public_key_file: &File,
) -> Result<String, AuthnErr> {
    // load the public key and serialize it as a JWK
    let public_key = rsa::read_public_key(public_key_file).await?;
    let jwk = rsa::rsa_public_key_to_jwk(&public_key);

    // build header and payload
    let header = JwtHeader {
        alg: "RS512",
        typ: "JWT",
        jwk,
    };
    let now = Utc::now();
    let exp = now + Duration::minutes(2);
    let payload = JwtPayload {
        jti: Uuid::new_v4().to_string(),
        iat: now.timestamp(),
        exp: exp.timestamp(),
    };

    // serialize header and payload, base64url-no-pad-encode, then join with '.'
    let signing_input = format!("{}.{}", jwt_segment(&header)?, jwt_segment(&payload)?);
    let signature = rsa::sign_rs512(private_key_file, signing_input.as_bytes()).await?;
    Ok(format!(
        "{signing_input}.{}",
        base64::encode_bytes_url_safe_no_pad(&signature),
    ))
}

/// Test-only shim exposing the internal `build_self_signed_jwt` helper to
/// integration tests under `agent/tests/`. Gated behind the `test` feature
/// so it is not part of the public release surface.
#[cfg(feature = "test")]
pub async fn build_self_signed_jwt_for_test(
    private_key_file: &File,
    public_key_file: &File,
) -> Result<String, AuthnErr> {
    build_self_signed_jwt(private_key_file, public_key_file).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::ser::Error as _;

    /// A `Serialize` impl that always fails — used to exercise the
    /// `jwt_segment` error mapping path.
    struct AlwaysFails;
    impl Serialize for AlwaysFails {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(S::Error::custom("intentional failure"))
        }
    }

    #[test]
    fn jwt_segment_maps_serialize_failure_to_serde_err() {
        let result = jwt_segment(&AlwaysFails);
        assert!(matches!(result, Err(AuthnErr::SerdeErr(_))));
    }
}
