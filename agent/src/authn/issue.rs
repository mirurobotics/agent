// internal crates
use crate::authn::{
    errors::{AuthnErr, SerdeErr, TimestampConversionErr},
    token::Token,
};
use crate::crypt::{base64, rsa};
use crate::filesys::file::File;
use crate::http::{self, devices};
use crate::trace;
use backend_api::models::{IssueDeviceClaims, IssueDeviceTokenRequest};

// external crates
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use uuid::Uuid;

#[derive(Serialize)]
struct IssueTokenClaim {
    pub device_id: String,
    pub nonce: String,
    pub expiration: i64,
}

/// Mint a fresh device token by signing claims with the on-disk private key
/// and posting them to the backend's `issue_token` endpoint.
///
/// This free function is the shared core used by both
/// `SingleThreadTokenManager::issue_token` (the long-running token refresh
/// path inside the agent) and `app::upgrade::reconcile` (the boot-time
/// rebootstrap path that does not yet have a `TokenManager` set up).
pub async fn issue_token<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    private_key_file: &File,
    device_id: &str,
) -> Result<Token, AuthnErr> {
    // prepare and sign the claims
    let payload = prepare_issue_token_request(private_key_file, device_id).await?;

    // send the token request
    let resp = devices::issue_token(
        http_client,
        devices::IssueTokenParams {
            id: device_id,
            payload: &payload,
        },
    )
    .await?;

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

async fn prepare_issue_token_request(
    private_key_file: &File,
    device_id: &str,
) -> Result<IssueDeviceTokenRequest, AuthnErr> {
    // prepare the claims
    let nonce = Uuid::new_v4().to_string();
    let expiration = Utc::now() + Duration::minutes(2);
    let claims = IssueTokenClaim {
        device_id: device_id.to_string(),
        nonce: nonce.clone(),
        expiration: expiration.timestamp(),
    };

    // serialize the claims into a JSON byte vector
    let claims_bytes = serde_json::to_vec(&claims).map_err(|e| {
        AuthnErr::SerdeErr(SerdeErr {
            source: e,
            trace: trace!(),
        })
    })?;

    // sign the claims
    let signature_bytes = rsa::sign(private_key_file, &claims_bytes).await?;
    let signature = base64::encode_bytes_standard(&signature_bytes);

    // convert it to the http client format
    let claims = IssueDeviceClaims {
        device_id: device_id.to_string(),
        nonce,
        expiration: expiration.to_rfc3339(),
    };

    Ok(IssueDeviceTokenRequest {
        claims: Box::new(claims),
        signature,
    })
}
