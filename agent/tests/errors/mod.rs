// internal crates
use miru_agent::errors::{self, Error};
use miru_agent::http::errors::{HTTPErr, MockErr, RequestFailed};
use miru_agent::http::request;

// external crates
use std::fmt;
use std::time::Duration;

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

#[test]
fn test_impl_error_macro_dispatch() {
    // MockErr uses trait defaults for code/http_status/params
    let mock = HTTPErr::MockErr(MockErr {
        is_network_connection_error: false,
    });
    assert_eq!(
        mock.code().as_str(),
        errors::Code::InternalServerError.as_str()
    );
    assert_eq!(
        mock.http_status(),
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    );
    assert!(mock.params().is_none());
    assert!(!mock.is_network_connection_error());

    // RequestFailed has custom implementations for code/http_status/params
    let request_failed = HTTPErr::RequestFailed(RequestFailed {
        request: request::Meta {
            url: "http://test".to_string(),
            method: reqwest::Method::GET,
            timeout: Duration::from_secs(1),
        },
        status: reqwest::StatusCode::BAD_REQUEST,
        error: None,
        trace: miru_agent::trace!(),
    });
    assert_eq!(
        request_failed.code().as_str(),
        errors::Code::InternalServerError.as_str()
    );
    assert_eq!(
        request_failed.http_status(),
        axum::http::StatusCode::BAD_REQUEST
    );
    assert!(request_failed.params().is_none());
    assert!(!request_failed.is_network_connection_error());
}
