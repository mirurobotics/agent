// internal crates
use miru_agent::errors::{Code, Error};
use miru_agent::events::errors::*;

// external crates
use axum::http::StatusCode;

// ========================= CURSOR EXPIRED ========================= //

mod cursor_expired {
    use super::*;

    fn cursor_expired_err() -> CursorExpiredErr {
        CursorExpiredErr {
            earliest_available: 50,
            requested: 10,
            trace: miru_agent::trace!(),
        }
    }

    #[test]
    fn code_is_cursor_expired() {
        let err = cursor_expired_err();
        assert!(matches!(err.code(), Code::CursorExpired));
    }

    #[test]
    fn http_status_is_gone() {
        let err = cursor_expired_err();
        assert_eq!(err.http_status(), StatusCode::GONE);
    }

    #[test]
    fn display_includes_cursor_values() {
        let err = cursor_expired_err();
        let msg = err.to_string();
        assert!(msg.contains("10"), "should contain requested cursor: {msg}");
        assert!(
            msg.contains("50"),
            "should contain earliest available: {msg}"
        );
    }
}

// ========================= MALFORMED CURSOR ========================= //

mod malformed_cursor {
    use super::*;

    fn malformed_cursor_err() -> MalformedCursorErr {
        MalformedCursorErr {
            trace: miru_agent::trace!(),
        }
    }

    #[test]
    fn code_is_malformed_cursor() {
        let err = malformed_cursor_err();
        assert!(matches!(err.code(), Code::MalformedCursor));
    }

    #[test]
    fn http_status_is_bad_request() {
        let err = malformed_cursor_err();
        assert_eq!(err.http_status(), StatusCode::BAD_REQUEST);
    }
}

// ========================= FROM CONVERSIONS ========================= //

mod from_conversions {
    use super::*;

    #[test]
    fn io_error_converts_to_events_err() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err: EventsErr = io_err.into();
        assert!(matches!(err, EventsErr::IoErr(_)));
    }

    #[test]
    fn serde_error_converts_to_events_err() {
        let serde_err = serde_json::from_str::<String>("not json").unwrap_err();
        let err: EventsErr = serde_err.into();
        assert!(matches!(err, EventsErr::SerializationErr(_)));
    }
}

// ========================= DELEGATION ========================= //

mod delegation {
    use super::*;

    #[test]
    fn events_err_delegates_code_for_cursor_expired() {
        let inner = CursorExpiredErr {
            earliest_available: 1,
            requested: 0,
            trace: miru_agent::trace!(),
        };
        let err = EventsErr::CursorExpiredErr(inner);
        assert!(matches!(err.code(), Code::CursorExpired));
    }

    #[test]
    fn events_err_delegates_http_status_for_malformed_cursor() {
        let inner = MalformedCursorErr {
            trace: miru_agent::trace!(),
        };
        let err = EventsErr::MalformedCursorErr(inner);
        assert_eq!(err.http_status(), StatusCode::BAD_REQUEST);
    }
}
