// internal crates
use crate::crypt::base64;
use crate::crypt::errors::{CryptErr, InvalidJWTErr, InvalidJWTPayloadFormatErr};
use crate::trace;

// external crates
use chrono::Utc;
use serde::{Deserialize, Serialize};

type DeviceID = String;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Claims {
    pub sub: String,
    pub iss: String,
    pub aud: String,
    pub exp: i64,
    pub iat: i64,
}

/// Decode a Miru JWT payload without verification
pub fn decode(token: &str) -> Result<Claims, CryptErr> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(CryptErr::InvalidJWTErr(InvalidJWTErr {
            msg: "Invalid JWT format".to_string(),
            trace: trace!(),
        }));
    }

    let payload = base64::decode_string_url_safe_no_pad(parts[1])?;
    let claims: Claims = serde_json::from_str(&payload).map_err(|e| {
        CryptErr::InvalidJWTPayloadErr(InvalidJWTPayloadFormatErr {
            msg: e.to_string(),
            trace: trace!(),
        })
    })?;

    Ok(claims)
}

pub fn extract_device_id(token: &str) -> Result<DeviceID, CryptErr> {
    let claims = decode(token)?;
    Ok(claims.sub)
}

/// Validate a claim's payload for a Miru JWT and returns the device_id
pub fn validate_claims(claim: Claims) -> Result<DeviceID, CryptErr> {
    if claim.iss != "miru" {
        return Err(CryptErr::InvalidJWTErr(InvalidJWTErr {
            msg: "Invalid issuer".to_string(),
            trace: trace!(),
        }));
    }

    if claim.aud != "device" {
        return Err(CryptErr::InvalidJWTErr(InvalidJWTErr {
            msg: "Invalid audience".to_string(),
            trace: trace!(),
        }));
    }

    // grant a 15 second tolerance for the issued at time (iat) field
    let iat_tol = 15;
    if claim.iat > Utc::now().timestamp() + iat_tol {
        return Err(CryptErr::InvalidJWTErr(InvalidJWTErr {
            msg: "Issued at time is in the future".to_string(),
            trace: trace!(),
        }));
    }

    if claim.exp < Utc::now().timestamp() {
        return Err(CryptErr::InvalidJWTErr(InvalidJWTErr {
            msg: "Expiration time is in the past".to_string(),
            trace: trace!(),
        }));
    }

    Ok(claim.sub)
}

/// Decode a Miru JWT payload and validate the claims. The "sub" field is the device
pub fn validate(token: &str) -> Result<String, CryptErr> {
    let claims = decode(token)?;
    validate_claims(claims)
}
