//! Report metadata: its conversion from the control plane's calibration.

// internal crates
use crate::test_utils::calibrations::{full_metadata, report_id};
use backend_api::models::{self as backend_client, BaseExternalUser};
use miru_daemon::calibrations::{Invalidation, ReportMetadata};

// external crates
use chrono::{DateTime, Utc};

/// An invalidation made an hour after a `full_metadata` report was received.
fn invalidation() -> Invalidation {
    Invalidation {
        invalidated_at: DateTime::from_timestamp(4600, 0).unwrap(),
        reason: "sensor was bumped".to_string(),
    }
}

/// `full_metadata(name)` marked invalid.
fn invalidated(name: &str) -> ReportMetadata {
    ReportMetadata {
        invalidation: Some(invalidation()),
        ..full_metadata(name)
    }
}

mod from_backend {
    use super::*;

    /// The control plane's calibration for `full_metadata("arm")`.
    fn calibration() -> backend_client::Calibration {
        backend_client::Calibration {
            id: "cal_1".to_string(),
            report_id: report_id("arm"),
            component_slug: "arm".to_string(),
            role_slug: Some("aligner-1".to_string()),
            aspect_slug: Some("pose".to_string()),
            serial_number: Some("SN-1".to_string()),
            reported_by: Some(Some(Box::new(BaseExternalUser {
                identifier: "Ada Lovelace".to_string(),
                ..BaseExternalUser::default()
            }))),
            deployment_id: Some("dpl_1".to_string()),
            format: backend_client::InstanceFormat::INSTANCE_FORMAT_YAML,
            reported_at: "1970-01-01T00:16:40Z".to_string(),
            ..backend_client::Calibration::default()
        }
    }

    #[test]
    fn maps_every_field() {
        assert_eq!(ReportMetadata::from(calibration()), full_metadata("arm"));
    }

    #[test]
    fn reported_by_is_none_when_not_expanded_or_null() {
        for reported_by in [None, Some(None)] {
            let found = ReportMetadata::from(backend_client::Calibration {
                reported_by,
                ..calibration()
            });
            assert_eq!(found.reported_by, None);
        }
    }

    #[test]
    fn maps_an_invalidation() {
        let found = ReportMetadata::from(backend_client::Calibration {
            invalidated_at: Some("1970-01-01T01:16:40Z".to_string()),
            invalidation_reason: Some("sensor was bumped".to_string()),
            ..calibration()
        });
        assert_eq!(found, invalidated("arm"));
    }

    #[test]
    fn a_missing_invalidation_reason_is_empty() {
        let found = ReportMetadata::from(backend_client::Calibration {
            invalidated_at: Some("1970-01-01T01:16:40Z".to_string()),
            ..calibration()
        });
        let expected = Invalidation {
            reason: String::new(),
            ..invalidation()
        };
        assert_eq!(found.invalidation, Some(expected));
    }

    #[test]
    fn a_reason_without_invalidated_at_is_valid() {
        let found = ReportMetadata::from(backend_client::Calibration {
            invalidation_reason: Some("sensor was bumped".to_string()),
            ..calibration()
        });
        assert_eq!(found.invalidation, None);
    }

    #[test]
    fn an_unparseable_invalidated_at_is_the_epoch() {
        let found = ReportMetadata::from(backend_client::Calibration {
            invalidated_at: Some("not a time".to_string()),
            invalidation_reason: Some("sensor was bumped".to_string()),
            ..calibration()
        });
        let expected = Invalidation {
            invalidated_at: DateTime::<Utc>::UNIX_EPOCH,
            ..invalidation()
        };
        assert_eq!(found.invalidation, Some(expected));
    }

    #[test]
    fn an_unparseable_reported_at_is_the_epoch() {
        let found = ReportMetadata::from(backend_client::Calibration {
            reported_at: "not a time".to_string(),
            ..calibration()
        });
        assert_eq!(found.reported_at, DateTime::<Utc>::UNIX_EPOCH);
    }
}
