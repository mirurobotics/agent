use miru_agent::errors::Error;
use miru_agent::http::errors::{MockErr, RequestFailed, ReqwestErr, ReqwestErrKind, TimeoutErr};
use miru_agent::http::request::{Meta, Params};
use openapi_client::models::{Error as ApiError, ErrorResponse};

fn meta() -> Meta {
    Params::get("http://test/errors").meta().unwrap()
}

fn trace() -> Box<miru_agent::errors::Trace> {
    miru_agent::trace!()
}

pub mod request_failed {
    use super::*;

    fn make_error_response(code: &str, message: &str) -> ErrorResponse {
        ErrorResponse::new(ApiError::new(
            code.to_string(),
            std::collections::HashMap::new(),
            message.to_string(),
        ))
    }

    #[test]
    fn display_with_error() {
        let err = RequestFailed {
            request: meta(),
            status: reqwest::StatusCode::NOT_FOUND,
            error: Some(make_error_response("not_found", "resource not found")),
            trace: trace(),
        };
        let display = format!("{err}");
        assert!(display.contains("404"));
        assert!(display.contains("resource not found"));
    }

    #[test]
    fn display_without_error() {
        let err = RequestFailed {
            request: meta(),
            status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            error: None,
            trace: trace(),
        };
        let display = format!("{err}");
        assert!(display.contains("500"));
        assert!(display.contains("unknown miru server error"));
    }

    #[test]
    fn code_delegates_to_error_response() {
        let err = RequestFailed {
            request: meta(),
            status: reqwest::StatusCode::NOT_FOUND,
            error: Some(make_error_response("not_found", "msg")),
            trace: trace(),
        };
        match err.code() {
            miru_agent::errors::Code::BackendError(code) => assert_eq!(code, "not_found"),
            other => panic!("expected BackendError, got: {other:?}"),
        }
    }

    #[test]
    fn code_without_error_is_internal_server_error() {
        let err = RequestFailed {
            request: meta(),
            status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            error: None,
            trace: trace(),
        };
        match err.code() {
            miru_agent::errors::Code::InternalServerError => {}
            other => panic!("expected InternalServerError, got: {other:?}"),
        }
    }

    #[test]
    fn http_status() {
        let err = RequestFailed {
            request: meta(),
            status: reqwest::StatusCode::BAD_REQUEST,
            error: None,
            trace: trace(),
        };
        assert_eq!(err.http_status(), reqwest::StatusCode::BAD_REQUEST);
    }

    #[test]
    fn params_returns_some_when_error_present() {
        let err = RequestFailed {
            request: meta(),
            status: reqwest::StatusCode::NOT_FOUND,
            error: Some(make_error_response("not_found", "msg")),
            trace: trace(),
        };
        assert!(err.params().is_some());
    }

    #[test]
    fn params_returns_none_when_no_error() {
        let err = RequestFailed {
            request: meta(),
            status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            error: None,
            trace: trace(),
        };
        assert!(err.params().is_none());
    }
}

pub mod timeout_err {
    use super::*;

    #[test]
    fn display_format() {
        let err = TimeoutErr {
            msg: "timed out".to_string(),
            request: meta(),
            trace: trace(),
        };
        let display = format!("{err}");
        assert!(display.contains("timed out"));
        assert!(display.contains("10 seconds"));
    }

    #[test]
    fn is_network_connection_error_returns_true() {
        let err = TimeoutErr {
            msg: "timed out".to_string(),
            request: meta(),
            trace: trace(),
        };
        assert!(err.is_network_connection_error());
    }
}

pub mod reqwest_err_kind {
    use super::*;

    // We can't easily construct a real reqwest::Error, so we test display via
    // the HTTPErr wrapper and test is_network_connection_error on ReqwestErrKind.

    #[test]
    fn connection_is_network_connection_error() {
        // Make a real reqwest::Error by connecting to a refused port
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt
            .block_on(async {
                reqwest::Client::new()
                    .get("http://127.0.0.1:1")
                    .send()
                    .await
            })
            .unwrap_err();

        let reqwest_err = ReqwestErr {
            kind: ReqwestErrKind::Connection,
            request: meta(),
            source: err,
            trace: trace(),
        };
        assert!(reqwest_err.is_network_connection_error());
        let display = format!("{reqwest_err}");
        assert!(display.contains("network connection error"));
    }

    #[test]
    fn other_is_not_network_connection_error() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt
            .block_on(async {
                reqwest::Client::new()
                    .get("http://127.0.0.1:1")
                    .send()
                    .await
            })
            .unwrap_err();

        let reqwest_err = ReqwestErr {
            kind: ReqwestErrKind::Other,
            request: meta(),
            source: err,
            trace: trace(),
        };
        assert!(!reqwest_err.is_network_connection_error());
    }
}

pub mod mock_err {
    use super::*;

    #[test]
    fn is_network_connection_error_delegates_to_field_true() {
        let err = MockErr {
            is_network_connection_error: true,
        };
        assert!(err.is_network_connection_error());
    }

    #[test]
    fn is_network_connection_error_delegates_to_field_false() {
        let err = MockErr {
            is_network_connection_error: false,
        };
        assert!(!err.is_network_connection_error());
    }

    #[test]
    fn display_format() {
        let err = MockErr {
            is_network_connection_error: true,
        };
        let display = format!("{err}");
        assert!(display.contains("true"));
    }
}
