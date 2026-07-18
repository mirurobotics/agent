pub mod harnesses;

// standard crates
use std::fmt;
use std::time::Duration;

// internal crates
use crate::errors::harnesses::{assert_error, Expected};
use miru_agent::errors::{self, Code, HTTPCode};
use miru_agent::http::errors::{MockErr, RequestFailed};
use miru_agent::http::request;
use miru_agent::http::HTTPErr;

/// Number of variants in errors::Code; keep in sync so every arm has a test case.
const EXPECTED_CODE_VARIANTS: usize = 5;

#[test]
fn test_code_as_str() {
    let cases: &[(errors::Code, &str)] = &[
        (errors::Code::InternalServerError, "internal_server_error"),
        (errors::Code::ResourceNotFound, "resource_not_found"),
        (errors::Code::CursorExpired, "cursor_expired"),
        (errors::Code::MalformedCursor, "malformed_cursor"),
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
        // Exhaustive match with no wildcard arm: adding a new `Code` variant
        // fails to compile here, forcing this case list (and
        // EXPECTED_CODE_VARIANTS) to be updated in lockstep.
        match code {
            errors::Code::InternalServerError
            | errors::Code::ResourceNotFound
            | errors::Code::CursorExpired
            | errors::Code::MalformedCursor
            | errors::Code::BackendError(_) => {}
        }
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
    assert_error(
        &err,
        Expected::new(Code::InternalServerError, HTTPCode::INTERNAL_SERVER_ERROR),
    );
}

#[test]
fn test_impl_error_macro_dispatch() {
    // MockErr uses trait defaults for code/http_status/params
    let mock = HTTPErr::MockErr(MockErr {
        is_network_conn_err: false,
    });
    assert_error(
        &mock,
        Expected::new(Code::InternalServerError, HTTPCode::INTERNAL_SERVER_ERROR),
    );

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
    assert_error(
        &request_failed,
        Expected::new(Code::InternalServerError, HTTPCode::BAD_REQUEST),
    );
}
