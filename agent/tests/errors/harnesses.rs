//! Reusable error-testing harness for the agent's `errors::Error` trait surface.
//!
//! Mirrors `agent/tests/models/harnesses.rs`: the actual assertions live in a
//! plain (non-`#[test]`) function, `assert_error`. Because that function carries
//! no `#[test]` / `#[tokio::test]` attribute, the repo's field-by-field-assert
//! lint (`tools/lint/src/asserts/detect.rs`, which only scans functions marked
//! `#[test]`) never inspects it. Modules that route their trait-surface
//! assertions through this harness are therefore immune to that lint by
//! construction and can drop their `// lint:allow(field-by-field-assert)`
//! suppressions.

// internal crates
use miru_agent::errors::{Code, Error, HTTPCode};

/// Expected trait-surface values for an `errors::Error`.
pub struct Expected {
    pub code: Code,
    pub http_status: HTTPCode,
    pub params: Option<serde_json::Value>,
    pub is_network_conn_err: bool,
}

impl Expected {
    /// Construct with the two always-present fields; optional expectations
    /// default to `None` params and a non-network-connection error.
    pub fn new(code: Code, http_status: HTTPCode) -> Self {
        Self {
            code,
            http_status,
            params: None,
            is_network_conn_err: false,
        }
    }

    /// Override the expected `params()`.
    pub fn with_params(mut self, params: serde_json::Value) -> Self {
        self.params = Some(params);
        self
    }

    /// Override the expected `is_network_conn_err()`.
    pub fn with_network_conn_err(mut self, is_network_conn_err: bool) -> Self {
        self.is_network_conn_err = is_network_conn_err;
        self
    }
}

/// Assert that `err` exposes the trait-surface values declared in `want`.
///
/// `code` is compared via `as_str()` because `errors::Code` does not derive
/// `PartialEq`. This is intentionally not a `#[test]` fn — see the module doc
/// for why that keeps the harness immune to the field-by-field-assert lint.
pub fn assert_error(err: &dyn Error, want: Expected) {
    assert_eq!(
        err.code().as_str(),
        want.code.as_str(),
        "code mismatch: got {:?}, want {:?}",
        err.code(),
        want.code,
    );
    assert_eq!(err.http_status(), want.http_status, "http_status mismatch");
    assert_eq!(err.params(), want.params, "params mismatch");
    assert_eq!(
        err.is_network_conn_err(),
        want.is_network_conn_err,
        "is_network_conn_err mismatch",
    );
}
