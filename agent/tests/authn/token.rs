// internal crates
use miru_agent::authn::token::Updates;
use miru_agent::authn::Token;
use miru_agent::models::Patch;

// external crates
use chrono::{DateTime, Duration, Utc};
use secrecy::{ExposeSecret, SecretString};
use serde_json::json;

#[test]
fn deserialize_token() {
    let expected = Token {
        token: SecretString::from("123"),
        expires_at: Utc::now(),
    };
    let valid_input = json!({
        "token": expected.token.expose_secret(),
        "expires_at": expected.expires_at,
    });
    let token: Token = serde_json::from_value(valid_input).unwrap();
    assert_eq!(token, expected);

    let empty_input = json!({});
    assert!(serde_json::from_value::<Token>(empty_input).is_err());

    // a non-string `token` fails inside the secret_string deserialize helper
    let non_string_token = json!({
        "token": 123,
        "expires_at": Utc::now(),
    });
    assert!(serde_json::from_value::<Token>(non_string_token).is_err());

    // all fields are required so don't test partial deserialization

    // invalid JSON
    assert!(serde_json::from_str::<Token>("invalid-json").is_err());
}

#[test]
fn serialize_token_uses_plain_string() {
    // The on-disk contract for token.json is a plain JSON string under `token`.
    let token = Token {
        token: SecretString::from("raw-jwt-value"),
        expires_at: DateTime::<Utc>::default(),
    };
    let value = serde_json::to_value(&token).unwrap();
    assert_eq!(value["token"], json!("raw-jwt-value"));
}

#[test]
fn token_serde_round_trips() {
    let original = Token {
        token: SecretString::from("round-trip-jwt"),
        expires_at: Utc::now(),
    };
    let serialized = serde_json::to_string(&original).unwrap();
    // the serialized JSON contains the raw token string under key `token`
    assert!(serialized.contains("\"token\":\"round-trip-jwt\""));

    let deserialized: Token = serde_json::from_str(&serialized).unwrap();
    assert_eq!(
        original.token.expose_secret(),
        deserialized.token.expose_secret()
    );
    assert_eq!(original.expires_at, deserialized.expires_at);
    assert_eq!(original, deserialized);
}

#[test]
fn debug_redacts_token() {
    let token = Token {
        token: SecretString::from("secret-value"),
        expires_at: Utc::now(),
    };
    let debug_output = format!("{:?}", token);
    assert!(debug_output.contains("[REDACTED]"));
    assert!(!debug_output.contains("secret-value"));
}

#[test]
fn token_update_empty() {
    let initial = Token {
        token: SecretString::from("123"),
        expires_at: Utc::now(),
    };
    let updates = Updates::empty();
    let expected = initial.clone();
    let mut actual = initial.clone();
    actual.patch(updates);
    assert_eq!(expected, actual);
}

#[test]
fn token_update_all() {
    let initial = Token {
        token: SecretString::from("123"),
        expires_at: Utc::now(),
    };
    let new_expiry = Utc::now() + Duration::days(1);
    let updates = Updates {
        token: Some(SecretString::from("456")),
        expires_at: Some(new_expiry),
    };
    let mut actual = initial.clone();
    actual.patch(updates);
    assert_eq!(actual.token.expose_secret(), "456");
    assert_eq!(actual.expires_at, new_expiry);
}

#[test]
fn token_partial_update() {
    let initial = Token {
        token: SecretString::from("old"),
        expires_at: Utc::now(),
    };
    let updates = Updates {
        token: Some(SecretString::from("new")),
        expires_at: None,
    };
    let mut actual = initial.clone();
    actual.patch(updates);
    assert_eq!(actual.token.expose_secret(), "new");
    assert_eq!(actual.expires_at, initial.expires_at);
}

#[test]
fn token_update_expires_at_only() {
    let initial = Token {
        token: SecretString::from("unchanged"),
        expires_at: Utc::now(),
    };
    let new_expiry = Utc::now() + Duration::days(30);
    let updates = Updates {
        token: None,
        expires_at: Some(new_expiry),
    };
    let mut actual = initial.clone();
    actual.patch(updates);
    assert_eq!(actual.token.expose_secret(), "unchanged");
    assert_eq!(actual.expires_at, new_expiry);
}

#[test]
fn partial_eq_detects_differences() {
    let base = Token {
        token: SecretString::from("same"),
        expires_at: DateTime::<Utc>::default(),
    };

    // differs only by token value (exercises the token comparison branch)
    let other_token = Token {
        token: SecretString::from("different"),
        expires_at: base.expires_at,
    };
    assert_ne!(base, other_token);

    // differs only by expiry (exercises the expires_at short-circuit branch)
    let other_expiry = Token {
        token: SecretString::from("same"),
        expires_at: base.expires_at + Duration::hours(1),
    };
    assert_ne!(base, other_expiry);
}

#[test]
fn is_expired_past() {
    let token = Token {
        token: SecretString::from("t"),
        expires_at: Utc::now() - Duration::seconds(1),
    };
    assert!(token.is_expired());
}

#[test]
fn is_expired_future() {
    let token = Token {
        token: SecretString::from("t"),
        expires_at: Utc::now() + Duration::hours(1),
    };
    assert!(!token.is_expired());
}

#[test]
fn default_token_is_expired() {
    let token = Token::default();
    assert_eq!(token.token.expose_secret(), "");
    assert_eq!(token.expires_at, DateTime::<Utc>::default());
    assert!(token.is_expired());
}
