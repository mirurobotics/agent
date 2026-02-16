// internal crates
use miru_agent::errors::{self, Error};

// external crates
use std::fmt;

/// Number of variants in errors::Code; keep in sync so every arm has a test case.
const EXPECTED_CODE_VARIANTS: usize = 3;

#[test]
fn test_code_as_str() {
    let cases: &[(errors::Code, &str)] = &[
        (errors::Code::InternalServerError, "internal_server_error"),
        (errors::Code::ResourceNotFound, "resource_not_found"),
        (
            errors::Code::BackendError("custom_code".to_string()),
            "custom_code",
        ),
    ];
    assert_eq!(
        cases.len(),
        EXPECTED_CODE_VARIANTS,
        "every Code variant must have exactly one test case; update EXPECTED_CODE_VARIANTS when adding variants"
    );
    for (code, expected) in cases {
        assert_eq!(code.as_str(), *expected, "Code::{:?}", code);
    }
}

#[derive(Debug)]
struct DefaultErr {}

impl fmt::Display for DefaultErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "default error")
    }
}

impl std::error::Error for DefaultErr {}

impl errors::Error for DefaultErr {}

#[test]
fn test_error_trait_defaults() {
    let err = DefaultErr {};
    assert_eq!(
        err.code().as_str(),
        errors::Code::InternalServerError.as_str()
    );
    assert_eq!(
        err.http_status(),
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    );
    assert!(err.params().is_none());
    assert!(!err.is_network_connection_error());
}
