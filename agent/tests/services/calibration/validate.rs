//! Intake validation against Miru's calibration rules.

// internal crates
use miru_agent::errors::{Error, HTTPCode};
use miru_agent::services::calibration::{
    validate::{self, is_slug, MAX_CONTENT_BYTES},
    CreateParams,
};
use miru_agent::services::errors::InvalidRequestErr;

// external crates
use serde_json::{json, Value};

fn params() -> CreateParams {
    CreateParams {
        component_slug: "arm".to_string(),
        role: None,
        aspect: None,
        serial_number: None,
        content: json!({ "format": "json", "data": "{}" }),
    }
}

fn with_content(content: Value) -> CreateParams {
    CreateParams {
        content,
        ..params()
    }
}

fn with_data(format: &str, data: String) -> CreateParams {
    with_content(json!({ "format": format, "data": data }))
}

/// Assert `p` is rejected as a 422 `invalid_request` naming `field`.
#[track_caller]
fn assert_rejects(p: &CreateParams, field: &str) {
    let err: InvalidRequestErr = match validate::params(p) {
        Ok(()) => panic!("expected {field} to be rejected: {p:?}"),
        Err(err) => err,
    };
    assert_eq!(err.code().as_str(), "invalid_request", "{err}");
    assert_eq!(err.http_status(), HTTPCode::UNPROCESSABLE_ENTITY, "{err}");
    assert_eq!(err.params(), Some(json!({ "field": field })), "{err}");
    assert!(
        err.to_string().starts_with(&format!("invalid {field}: ")),
        "{err}"
    );
}

#[test]
fn accepts_a_minimal_report() {
    validate::params(&params()).unwrap();
}

#[test]
fn accepts_every_optional_field() {
    let p = CreateParams {
        role: Some("left".to_string()),
        aspect: Some("pose".to_string()),
        serial_number: Some("SN-1 (rev. B)".to_string()),
        ..params()
    };

    validate::params(&p).unwrap();
}

#[test]
fn accepts_every_format() {
    for format in ["json", "yaml", "jsonc", "xml", "text"] {
        let p = with_data(format, "1".to_string());
        assert!(validate::params(&p).is_ok(), "{format}");
    }
}

#[test]
fn accepts_valid_slugs() {
    let max = "a".repeat(32);
    for slug in [
        "arm",
        "aligner-1",
        "front-left",
        "home-offsets",
        "0",
        "a--b",
        &max,
    ] {
        assert!(is_slug(slug), "{slug}");
    }
}

#[test]
fn rejects_bad_slugs_in_each_slug_field() {
    let too_long = "a".repeat(33);
    let bad = [
        "", "Arm", "-arm", "arm-", "-", " arm", "arm 1", "arm_1", "ärm", &too_long,
    ];
    for slug in bad {
        assert!(!is_slug(slug), "{slug:?}");
        let s = Some(slug.to_string());
        let component = CreateParams {
            component_slug: slug.to_string(),
            ..params()
        };
        assert_rejects(&component, "component_slug");
        assert_rejects(
            &CreateParams {
                role: s.clone(),
                ..params()
            },
            "role",
        );
        assert_rejects(
            &CreateParams {
                aspect: s,
                ..params()
            },
            "aspect",
        );
    }
}

#[test]
fn content_of_exactly_1_mib_is_accepted() {
    let p = with_data("text", "x".repeat(MAX_CONTENT_BYTES));

    validate::params(&p).unwrap();
}

#[test]
fn content_of_1_mib_plus_1_byte_is_rejected() {
    assert_rejects(
        &with_data("text", "x".repeat(MAX_CONTENT_BYTES + 1)),
        "content.data",
    );
}

#[test]
fn content_size_counts_bytes() {
    // 2 bytes each: half as many characters as the byte limit still fills it
    let at_limit = "é".repeat(MAX_CONTENT_BYTES / 2);
    validate::params(&with_data("text", at_limit.clone())).unwrap();

    assert_rejects(&with_data("text", at_limit + "x"), "content.data");
}

#[test]
fn empty_data_is_rejected() {
    assert_rejects(&with_data("text", String::new()), "content.data");
}

#[test]
fn serial_number_limit_counts_characters() {
    let at_limit = CreateParams {
        serial_number: Some("é".repeat(255)),
        ..params()
    };
    validate::params(&at_limit).unwrap();

    let over = CreateParams {
        serial_number: Some("é".repeat(256)),
        ..params()
    };
    assert_rejects(&over, "serial_number");
}

#[test]
fn serial_number_rejects_nul() {
    let p = CreateParams {
        serial_number: Some("SN\u{0}1".to_string()),
        ..params()
    };

    assert_rejects(&p, "serial_number");
}

#[test]
fn rejects_malformed_content() {
    let cases = [
        json!("{}"),
        json!({ "format": "json" }),
        json!({ "format": "json", "data": 1 }),
        json!({ "format": 1, "data": "{}" }),
    ];
    for content in cases {
        assert_rejects(&with_content(content), "content");
    }
}

#[test]
fn rejects_an_unknown_format() {
    assert_rejects(&with_data("toml", "a = 1".to_string()), "content.format");
}

#[test]
fn rejects_unparseable_json() {
    for data in ["{", "{\"a\": 1} trailing", "not json"] {
        assert_rejects(&with_data("json", data.to_string()), "content.data");
        // only json is parsed; Miru checks yaml and jsonc
        for format in ["text", "xml", "yaml", "jsonc"] {
            assert!(validate::params(&with_data(format, data.to_string())).is_ok());
        }
    }
}

#[test]
fn accepts_deeply_nested_json() {
    // the syntax check has no 128-level limit (Go's encoding/json allows
    // 10,000 levels)
    let deep = format!("{}{}", "[".repeat(1000), "]".repeat(1000));

    validate::params(&with_data("json", deep)).unwrap();
}

#[test]
fn checks_fields_in_order() {
    let p = CreateParams {
        component_slug: "Bad".to_string(),
        role: Some("Bad".to_string()),
        serial_number: Some("\u{0}".to_string()),
        content: json!(null),
        ..params()
    };

    assert_rejects(&p, "component_slug");
}
