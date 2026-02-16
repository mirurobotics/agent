// internal crates
use miru_agent::authn::token::{Token, Updates};
use miru_agent::models::Mergeable;

// external crates
use chrono::{Duration, Utc};
use serde_json::json;
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};

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
