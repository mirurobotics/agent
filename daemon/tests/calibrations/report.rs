//! Report metadata: its conversion from the control plane's calibration.

// internal crates
use crate::test_utils::calibrations::report_id;
use backend_api::models::{self as backend_client, BaseExternalUser};
use miru_daemon::calibrations::ReportMetadata;
use miru_daemon::models::InstanceFormat;

// external crates
use chrono::{DateTime, Utc};

/// Deterministic report metadata, ready the moment it arrives.
pub fn metadata(name: &str) -> ReportMetadata {
    ReportMetadata {
        report_id: report_id(name),
        component: name.to_string(),
        format: InstanceFormat::Yaml,
        role: Some("aligner-1".to_string()),
        aspect: Some("pose".to_string()),
        serial_number: Some("SN-1".to_string()),
        reported_by: Some("Ada Lovelace".to_string()),
        deployment_id: Some("dpl_1".to_string()),
        reported_at: DateTime::from_timestamp(1000, 0).unwrap(),
    }
}

mod from_backend {
    use super::*;

    /// Miru's calibration for `metadata("arm")`.
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
        assert_eq!(ReportMetadata::from(calibration()), metadata("arm"));
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
    fn an_unparseable_reported_at_is_the_epoch() {
        let found = ReportMetadata::from(backend_client::Calibration {
            reported_at: "not a time".to_string(),
            ..calibration()
        });
        assert_eq!(found.reported_at, DateTime::<Utc>::UNIX_EPOCH);
    }
}
