// internal crates
use crate::authn::errors::{AuthnErr, SerdeErr, TimestampConversionErr};
use crate::authn::token::Token;
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

pub async fn issue_token(
    http_client: &impl http::ClientI,
    private_key_file: &File,
    public_key_file: &File,
) -> Result<Token, AuthnErr> {
    // build the self-signed JWT
    let jwt = mint_jwt(private_key_file, public_key_file).await?;

    // send the token request
    let params = devices::IssueTokenParams { token: &jwt };
    let resp = devices::issue_token(http_client, params).await?;

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

/// Build a self-signed RS512 JWT (RFC 7519) whose header carries the device's public
/// key as a JWK (RFC 7517). The payload contains a unique `jti`, the current `iat`, and
/// an `exp` two minutes in the future.
pub(crate) async fn mint_jwt(
    private_key_file: &File,
    public_key_file: &File,
) -> Result<String, AuthnErr> {
    // load the public key and serialize it as a JWK
    let public_key = rsa::read_public_key(public_key_file).await?;
    let jwk = rsa::pub_key_to_jwk(&public_key);

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
    let signing_input = format!("{}.{}", encode_part(&header)?, encode_part(&payload)?);
    let signature = rsa::sign_rs512(private_key_file, signing_input.as_bytes()).await?;
    Ok(format!(
        "{signing_input}.{}",
        base64::encode_bytes_url_safe_no_pad(&signature),
    ))
}

fn encode_part<T: Serialize>(value: &T) -> Result<String, AuthnErr> {
    let bytes = serde_json::to_vec(value).map_err(|e| {
        AuthnErr::SerdeErr(SerdeErr {
            source: e,
            trace: trace!(),
        })
    })?;
    Ok(base64::encode_bytes_url_safe_no_pad(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::filesys::{self, Overwrite};

    use openssl::hash::MessageDigest;
    use openssl::pkey::PKey;
    use openssl::sign::Verifier;
    use serde::ser::Error as _;
    use serde_json::Value;

    /// A `Serialize` impl that always fails — used to exercise the
    /// `encode_part` error mapping path.
    struct AlwaysFails;
    impl Serialize for AlwaysFails {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(S::Error::custom("intentional failure"))
        }
    }

    /// Generate a real RSA key pair in a temp dir and return the file handles.
    async fn generate_keys() -> (filesys::Dir, filesys::File, filesys::File) {
        let dir = filesys::Dir::create_temp_dir("authn_issue_test")
            .await
            .unwrap();
        let private_key_file = dir.file("private_key.pem");
        let public_key_file = dir.file("public_key.pem");
        rsa::gen_key_pair(2048, &private_key_file, &public_key_file, Overwrite::Allow)
            .await
            .unwrap();
        (dir, private_key_file, public_key_file)
    }

    #[test]
    fn encode_part_maps_serialize_failure_to_serde_err() {
        let result = encode_part(&AlwaysFails);
        assert!(matches!(result, Err(AuthnErr::SerdeErr(_))));
    }

    #[tokio::test]
    async fn mint_jwt_has_three_parts() {
        let (_dir, private_key_file, public_key_file) = generate_keys().await;
        let jwt = mint_jwt(&private_key_file, &public_key_file).await.unwrap();

        assert_eq!(3, jwt.split('.').count());
    }

    #[tokio::test]
    async fn mint_jwt_header_decodes_to_rs512_with_jwk() {
        let (_dir, private_key_file, public_key_file) = generate_keys().await;
        let jwt = mint_jwt(&private_key_file, &public_key_file).await.unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        let header_bytes = base64::decode_bytes_url_safe_no_pad(parts[0]).unwrap();
        let header: Value = serde_json::from_slice(&header_bytes).unwrap();

        assert_eq!("RS512", header["alg"]);
        assert_eq!("JWT", header["typ"]);
        assert_eq!("RSA", header["jwk"]["kty"]);
        assert!(!header["jwk"]["n"].as_str().unwrap().is_empty());
        assert!(!header["jwk"]["e"].as_str().unwrap().is_empty());
    }

    #[tokio::test]
    async fn mint_jwt_payload_decodes_with_jti_iat_exp() {
        let (_dir, private_key_file, public_key_file) = generate_keys().await;
        let before = Utc::now().timestamp();
        let jwt = mint_jwt(&private_key_file, &public_key_file).await.unwrap();
        let after = Utc::now().timestamp();
        let parts: Vec<&str> = jwt.split('.').collect();
        let payload_bytes = base64::decode_bytes_url_safe_no_pad(parts[1]).unwrap();
        let payload: Value = serde_json::from_slice(&payload_bytes).unwrap();

        let jti = payload["jti"].as_str().unwrap();
        assert!(!jti.is_empty());
        Uuid::parse_str(jti).unwrap();

        let iat = payload["iat"].as_i64().unwrap();
        assert!(
            iat >= before && iat <= after,
            "iat={iat} outside [{before}, {after}]"
        );

        let exp = payload["exp"].as_i64().unwrap();
        assert_eq!(iat + 120, exp);
    }

    #[tokio::test]
    async fn mint_jwt_signature_verifies_with_public_key() {
        let (_dir, private_key_file, public_key_file) = generate_keys().await;
        let jwt = mint_jwt(&private_key_file, &public_key_file).await.unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        let signing_input = format!("{}.{}", parts[0], parts[1]);
        let signature = base64::decode_bytes_url_safe_no_pad(parts[2]).unwrap();

        let public_key_rsa = rsa::read_public_key(&public_key_file).await.unwrap();
        let pkey = PKey::from_rsa(public_key_rsa).unwrap();
        let mut verifier = Verifier::new(MessageDigest::sha512(), &pkey).unwrap();
        verifier.update(signing_input.as_bytes()).unwrap();
        assert!(verifier.verify(&signature).unwrap());

        let mut tampered = signing_input.clone().into_bytes();
        tampered[0] ^= 0x01;
        let public_key_rsa = rsa::read_public_key(&public_key_file).await.unwrap();
        let pkey = PKey::from_rsa(public_key_rsa).unwrap();
        let mut verifier = Verifier::new(MessageDigest::sha512(), &pkey).unwrap();
        verifier.update(&tampered).unwrap();
        assert!(!verifier.verify(&signature).unwrap());
    }

    #[tokio::test]
    async fn mint_jwt_generates_unique_jti_across_calls() {
        let (_dir, private_key_file, public_key_file) = generate_keys().await;
        let jwt_a = mint_jwt(&private_key_file, &public_key_file).await.unwrap();
        let jwt_b = mint_jwt(&private_key_file, &public_key_file).await.unwrap();

        let parts_a: Vec<&str> = jwt_a.split('.').collect();
        let parts_b: Vec<&str> = jwt_b.split('.').collect();
        let payload_a: Value =
            serde_json::from_slice(&base64::decode_bytes_url_safe_no_pad(parts_a[1]).unwrap())
                .unwrap();
        let payload_b: Value =
            serde_json::from_slice(&base64::decode_bytes_url_safe_no_pad(parts_b[1]).unwrap())
                .unwrap();

        assert_ne!(payload_a["jti"], payload_b["jti"]);
    }

    #[tokio::test]
    async fn mint_jwt_returns_err_when_public_key_file_is_missing() {
        let dir = filesys::Dir::create_temp_dir("authn_issue_test")
            .await
            .unwrap();
        let private_key_file = dir.file("private_key.pem");
        let public_key_file = dir.file("public_key.pem");
        rsa::gen_key_pair(2048, &private_key_file, &public_key_file, Overwrite::Allow)
            .await
            .unwrap();
        public_key_file.delete().await.unwrap();

        let result = mint_jwt(&private_key_file, &public_key_file).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn mint_jwt_returns_err_when_private_key_file_is_missing() {
        let dir = filesys::Dir::create_temp_dir("authn_issue_test")
            .await
            .unwrap();
        let private_key_file = dir.file("private_key.pem");
        let public_key_file = dir.file("public_key.pem");
        rsa::gen_key_pair(2048, &private_key_file, &public_key_file, Overwrite::Allow)
            .await
            .unwrap();
        private_key_file.delete().await.unwrap();

        let result = mint_jwt(&private_key_file, &public_key_file).await;

        assert!(result.is_err());
    }
}
