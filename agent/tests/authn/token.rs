// internal crates
use miru_agent::authn::token::{Token, Updates};
use miru_agent::models::Mergeable;

// external crates
use chrono::{DateTime, Duration, Utc};
use serde_json::json;

#[test]
fn deserialize_token() {
    let expected = Token {
        token: "123".to_string(),
        expires_at: Utc::now(),
    };
    let valid_input = json!({
        "token": expected.token,
        "expires_at": expected.expires_at,
    });
    let token: Token = serde_json::from_value(valid_input).unwrap();
    assert_eq!(token, expected);

    let empty_input = json!({});
    assert!(serde_json::from_value::<Token>(empty_input).is_err());

    // all fields are required so don't test partial deserialization

    // invalid JSON
    assert!(serde_json::from_str::<Token>("invalid-json").is_err());
}

#[test]
fn token_merge_empty() {
    let initial = Token {
        token: "123".to_string(),
        expires_at: Utc::now(),
    };
    let updates = Updates::empty();
    let expected = initial.clone();
    let mut actual = initial.clone();
    actual.merge(updates);
    assert_eq!(expected, actual);
}

#[test]
fn debug_redacts_token() {
    let token = Token {
        token: "secret-value".to_string(),
        expires_at: Utc::now(),
    };
    let debug_output = format!("{:?}", token);
    assert!(debug_output.contains("[REDACTED]"));
    assert!(!debug_output.contains("secret-value"));
}

#[test]
fn token_merge_all() {
    let initial = Token {
        token: "123".to_string(),
        expires_at: Utc::now(),
    };
    let updates = Updates {
        token: Some("456".to_string()),
        expires_at: Some(Utc::now() + Duration::days(1)),
    };
    let expected = Token {
        token: updates.token.clone().unwrap(),
        expires_at: updates.expires_at.unwrap(),
    };
    let mut actual = initial.clone();
    actual.merge(updates);
    assert_eq!(expected, actual);
}

#[test]
fn is_expired_past() {
    let token = Token {
        token: "t".to_string(),
        expires_at: Utc::now() - Duration::seconds(1),
    };
    assert!(token.is_expired());
}

#[test]
fn is_expired_future() {
    let token = Token {
        token: "t".to_string(),
        expires_at: Utc::now() + Duration::hours(1),
    };
    assert!(!token.is_expired());
}

#[test]
fn default_token_is_expired() {
    let token = Token::default();
    assert_eq!(token.token, "");
    assert_eq!(token.expires_at, DateTime::<Utc>::default());
    assert!(token.is_expired());
}

#[test]
fn token_merge_partial_token_only() {
    let initial = Token {
        token: "old".to_string(),
        expires_at: Utc::now(),
    };
    let updates = Updates {
        token: Some("new".to_string()),
        expires_at: None,
    };
    let mut actual = initial.clone();
    actual.merge(updates);
    assert_eq!(actual.token, "new");
    assert_eq!(actual.expires_at, initial.expires_at);
}

#[test]
fn token_merge_partial_expires_at_only() {
    let initial = Token {
        token: "unchanged".to_string(),
        expires_at: Utc::now(),
    };
    let new_expiry = Utc::now() + Duration::days(30);
    let updates = Updates {
        token: None,
        expires_at: Some(new_expiry),
    };
    let mut actual = initial.clone();
    actual.merge(updates);
    assert_eq!(actual.token, "unchanged");
    assert_eq!(actual.expires_at, new_expiry);
}
