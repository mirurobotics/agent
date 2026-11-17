//! Report metadata: its conversion from the control plane's calibration.

// internal crates
use crate::test_utils::calibrations::{full_metadata, report_id};
use backend_api::models::{self as backend_client, BaseExternalUser};
use miru_daemon::calibrations::ReportMetadata;

// external crates
use chrono::{DateTime, Utc};

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
    fn maps_invalidated_at() {
        let found = ReportMetadata::from(backend_client::Calibration {
            invalidated_at: Some("1970-01-01T01:16:40Z".to_string()),
            invalidation_reason: Some("sensor was bumped".to_string()),
            ..calibration()
        });
        let expected = ReportMetadata {
            invalidated_at: Some(DateTime::from_timestamp(4600, 0).unwrap()),
            ..full_metadata("arm")
        };
        assert_eq!(found, expected);
    }

    #[test]
    fn an_unparseable_invalidated_at_is_the_epoch() {
        let found = ReportMetadata::from(backend_client::Calibration {
            invalidated_at: Some("not a time".to_string()),
            ..calibration()
        });
        assert_eq!(found.invalidated_at, Some(DateTime::<Utc>::UNIX_EPOCH));
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
