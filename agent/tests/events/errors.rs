// internal crates
use crate::errors::harnesses::{assert_error, Expected};
use miru_agent::errors::Code;
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
    fn cursor_expired_error_trait_surface() {
        let err = cursor_expired_err();
        assert_error(&err, Expected::new(Code::CursorExpired, StatusCode::GONE));
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
    fn malformed_cursor_error_trait_surface() {
        let err = malformed_cursor_err();
        assert_error(
            &err,
            Expected::new(Code::MalformedCursor, StatusCode::BAD_REQUEST),
        );
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
    fn events_err_delegates_trait_surface_for_cursor_expired() {
        let inner = CursorExpiredErr {
            earliest_available: 1,
            requested: 0,
            trace: miru_agent::trace!(),
        };
        let err = EventsErr::CursorExpiredErr(inner);
        assert_error(&err, Expected::new(Code::CursorExpired, StatusCode::GONE));
    }

    #[test]
    fn events_err_delegates_trait_surface_for_malformed_cursor() {
        let inner = MalformedCursorErr {
            trace: miru_agent::trace!(),
        };
        let err = EventsErr::MalformedCursorErr(inner);
        assert_error(
            &err,
            Expected::new(Code::MalformedCursor, StatusCode::BAD_REQUEST),
        );
    }
}
