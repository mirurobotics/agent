// internal crates
use miru_agent::authn::issue::build_self_signed_jwt_for_test;
use miru_agent::crypt::{base64, rsa};
use miru_agent::filesys::{self, Overwrite, PathExt};

// external crates
use chrono::Utc;
use openssl::hash::MessageDigest;
use openssl::pkey::PKey;
use openssl::sign::Verifier;
use serde_json::Value;
use uuid::Uuid;

/// Generate a real RSA key pair in a temp dir and return the file handles.
async fn generate_keys() -> (filesys::Dir, filesys::File, filesys::File) {
    let dir = filesys::Dir::create_temp_dir("authn_issue_test")
        .await
        .unwrap();
    let private_key_file = filesys::File::new(dir.path().join("private_key.pem"));
    let public_key_file = filesys::File::new(dir.path().join("public_key.pem"));
    rsa::gen_key_pair(2048, &private_key_file, &public_key_file, Overwrite::Allow)
        .await
        .unwrap();
    (dir, private_key_file, public_key_file)
}

#[tokio::test]
async fn jwt_has_three_segments() {
    let (_dir, private_key_file, public_key_file) = generate_keys().await;
    let jwt = build_self_signed_jwt_for_test(&private_key_file, &public_key_file)
        .await
        .unwrap();
    assert_eq!(jwt.split('.').count(), 3);
}

#[tokio::test]
async fn jwt_header_decodes_to_rs512_with_jwk() {
    let (_dir, private_key_file, public_key_file) = generate_keys().await;
    let jwt = build_self_signed_jwt_for_test(&private_key_file, &public_key_file)
        .await
        .unwrap();
    let parts: Vec<&str> = jwt.split('.').collect();
    let header_bytes = base64::decode_bytes_url_safe_no_pad(parts[0]).unwrap();
    let header: Value = serde_json::from_slice(&header_bytes).unwrap();

    assert_eq!(header["alg"], "RS512");
    assert_eq!(header["typ"], "JWT");
    assert_eq!(header["jwk"]["kty"], "RSA");
    assert!(header["jwk"]["n"].as_str().unwrap().len() > 0);
    assert!(header["jwk"]["e"].as_str().unwrap().len() > 0);
}

#[tokio::test]
async fn jwt_payload_decodes_with_jti_iat_exp() {
    let (_dir, private_key_file, public_key_file) = generate_keys().await;
    let before = Utc::now().timestamp();
    let jwt = build_self_signed_jwt_for_test(&private_key_file, &public_key_file)
        .await
        .unwrap();
    let after = Utc::now().timestamp();
    let parts: Vec<&str> = jwt.split('.').collect();
    let payload_bytes = base64::decode_bytes_url_safe_no_pad(parts[1]).unwrap();
    let payload: Value = serde_json::from_slice(&payload_bytes).unwrap();

    // jti is a non-empty UUID string
    let jti = payload["jti"].as_str().unwrap();
    assert!(!jti.is_empty());
    Uuid::parse_str(jti).unwrap();

    // iat is within the timestamp window observed during the call
    let iat = payload["iat"].as_i64().unwrap();
    assert!(
        iat >= before && iat <= after,
        "iat={iat} outside [{before}, {after}]"
    );

    // exp is iat + 120s
    let exp = payload["exp"].as_i64().unwrap();
    assert_eq!(exp, iat + 120);
}

#[tokio::test]
async fn jwt_signature_verifies_with_public_key() {
    let (_dir, private_key_file, public_key_file) = generate_keys().await;
    let jwt = build_self_signed_jwt_for_test(&private_key_file, &public_key_file)
        .await
        .unwrap();
    let parts: Vec<&str> = jwt.split('.').collect();
    let signing_input = format!("{}.{}", parts[0], parts[1]);
    let signature = base64::decode_bytes_url_safe_no_pad(parts[2]).unwrap();

    // Verify using the device's public key with SHA-512
    let public_key_rsa = rsa::read_public_key(&public_key_file).await.unwrap();
    let pkey = PKey::from_rsa(public_key_rsa).unwrap();
    let mut verifier = Verifier::new(MessageDigest::sha512(), &pkey).unwrap();
    verifier.update(signing_input.as_bytes()).unwrap();
    assert!(verifier.verify(&signature).unwrap());

    // Tampering with the signing input fails verification
    let mut tampered = signing_input.clone().into_bytes();
    tampered[0] ^= 0x01;
    let public_key_rsa = rsa::read_public_key(&public_key_file).await.unwrap();
    let pkey = PKey::from_rsa(public_key_rsa).unwrap();
    let mut verifier = Verifier::new(MessageDigest::sha512(), &pkey).unwrap();
    verifier.update(&tampered).unwrap();
    assert!(!verifier.verify(&signature).unwrap());
}

#[tokio::test]
async fn jti_is_unique_across_calls() {
    let (_dir, private_key_file, public_key_file) = generate_keys().await;
    let jwt_a = build_self_signed_jwt_for_test(&private_key_file, &public_key_file)
        .await
        .unwrap();
    let jwt_b = build_self_signed_jwt_for_test(&private_key_file, &public_key_file)
        .await
        .unwrap();

    let parts_a: Vec<&str> = jwt_a.split('.').collect();
    let parts_b: Vec<&str> = jwt_b.split('.').collect();
    let payload_a: Value = serde_json::from_slice(
        &base64::decode_bytes_url_safe_no_pad(parts_a[1]).unwrap(),
    )
    .unwrap();
    let payload_b: Value = serde_json::from_slice(
        &base64::decode_bytes_url_safe_no_pad(parts_b[1]).unwrap(),
    )
    .unwrap();

    assert_ne!(payload_a["jti"], payload_b["jti"]);
}

#[tokio::test]
async fn missing_public_key_file_returns_err() {
    let dir = filesys::Dir::create_temp_dir("authn_issue_test")
        .await
        .unwrap();
    let private_key_file = filesys::File::new(dir.path().join("private_key.pem"));
    let public_key_file = filesys::File::new(dir.path().join("public_key.pem"));
    rsa::gen_key_pair(2048, &private_key_file, &public_key_file, Overwrite::Allow)
        .await
        .unwrap();
    public_key_file.delete().await.unwrap();

    let result = build_self_signed_jwt_for_test(&private_key_file, &public_key_file).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn missing_private_key_file_returns_err() {
    let dir = filesys::Dir::create_temp_dir("authn_issue_test")
        .await
        .unwrap();
    let private_key_file = filesys::File::new(dir.path().join("private_key.pem"));
    let public_key_file = filesys::File::new(dir.path().join("public_key.pem"));
    rsa::gen_key_pair(2048, &private_key_file, &public_key_file, Overwrite::Allow)
        .await
        .unwrap();
    private_key_file.delete().await.unwrap();

    let result = build_self_signed_jwt_for_test(&private_key_file, &public_key_file).await;
    assert!(result.is_err());
}
