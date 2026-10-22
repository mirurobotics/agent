// internal crates
use crate::authn::{
    errors::{AuthnErr, SerdeErr, TimestampConversionErr},
    token::Token,
};
use crate::crypt::{base64, rsa};
use crate::filesys::file::File;
use crate::http::{self, devices};
use crate::trace;
use backend_api::models::TokenResponse;

// external crates
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use tracing::debug;
use uuid::Uuid;

#[derive(Serialize)]
struct JwtHeader {
    alg: &'static str,
    typ: &'static str,
    kid: String,
}

#[derive(Serialize)]
struct JwtPayload {
    jti: String,
    iat: i64,
    exp: i64,
}

pub async fn issue_token(
    http_client: &impl http::ClientI,
    private_key_file: &File,
    public_key_file: &File,
) -> Result<Token, AuthnErr> {
    let clock = http_client.clock();
    let server_time_before = clock.server_now();
    let resp = match request_token(http_client, private_key_file, public_key_file).await {
        Ok(resp) => resp,
        // A robot that booted with its clock at 1970 signs a JWT the backend
        // rejects as expired. Before the first backend response the agent has
        // no better time to sign with; the rejection itself carries the
        // server's time, so retry once now that it is known.
        Err(AuthnErr::HTTPErr(e))
            if is_unauthorized(&e)
                && server_time_before.is_none()
                && clock.server_now().is_some() =>
        {
            debug!("token request rejected before server time was known; retrying");
            request_token(http_client, private_key_file, public_key_file).await?
        }
        Err(e) => return Err(e),
    };

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

/// Sign a JWT with the server's time when known, else the system clock, and
/// send the token request.
async fn request_token(
    http_client: &impl http::ClientI,
    private_key_file: &File,
    public_key_file: &File,
) -> Result<TokenResponse, AuthnErr> {
    let now = http_client.clock().server_or_system_now();
    let jwt = mint_jwt(private_key_file, public_key_file, now).await?;
    let params = devices::IssueTokenParams { token: &jwt };
    Ok(devices::issue_token(http_client, params).await?)
}

fn is_unauthorized(e: &http::HTTPErr) -> bool {
    matches!(e, http::HTTPErr::RequestFailed(f) if f.status == reqwest::StatusCode::UNAUTHORIZED)
}

/// Build a self-signed RS512 JWT (RFC 7519) whose header carries the device's public
/// key fingerprint as `kid` (RFC 7515 §4.1.4). The backend looks up the enrolled device
/// by this fingerprint and verifies the signature with the stored public key. The
/// payload contains a unique `jti`, `iat` set to `now`, and an `exp` two minutes after
/// `now`. Pass the server's time when known: the backend rejects a JWT whose `exp`
/// has passed by its own clock.
pub async fn mint_jwt(
    private_key_file: &File,
    public_key_file: &File,
    now: DateTime<Utc>,
) -> Result<String, AuthnErr> {
    // load the public key and compute its canonical fingerprint
    let public_key = rsa::read_public_key(public_key_file).await?;
    let kid = rsa::fingerprint(&public_key)?;

    // build header and payload
    let header = JwtHeader {
        alg: "RS512",
        typ: "JWT",
        kid,
    };
    let exp = now + Duration::minutes(2);
    let payload = JwtPayload {
        jti: Uuid::new_v4().to_string(),
        iat: now.timestamp(),
        exp: exp.timestamp(),
    };

    // serialize header and payload, base64url-no-pad-encode, then join with '.'
    let signing_input = format!("{}.{}", encode_part(&header)?, encode_part(&payload)?);
    let signature = rsa::sign_rs512(private_key_file, signing_input.as_bytes()).await?;
    Ok(format!(
        "{signing_input}.{}",
        base64::encode_bytes_url_safe_no_pad(&signature),
    ))
}

pub fn encode_part<T: Serialize>(value: &T) -> Result<String, AuthnErr> {
    let bytes = serde_json::to_vec(value).map_err(|e| {
        AuthnErr::SerdeErr(SerdeErr {
            source: e,
            trace: trace!(),
        })
    })?;
    Ok(base64::encode_bytes_url_safe_no_pad(&bytes))
}
