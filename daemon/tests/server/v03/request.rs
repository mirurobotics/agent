// internal crates
use device_api_v03::models as openapi;
use miru_daemon::calibrations::Filter;
use miru_daemon::models::InstanceFormat;
use miru_daemon::server::v03::request::{CreateCalibrationBody, LatestCalibrationsQuery};
use miru_daemon::services::calibration::CreateParams;

fn calibration_request(format: openapi::InstanceFormat) -> openapi::CreateCalibrationRequest {
    openapi::CreateCalibrationRequest {
        component: "arm-left".into(),
        serial_number: Some("SN-1".into()),
        reported_by: Some("Ada Lovelace".into()),
        content: Box::new(openapi::InstanceContent::new(format, "offset: 1\n".into())),
    }
}

pub mod create_calibration_request {
    use super::*;

    #[test]
    fn converts_every_field() {
        let request = calibration_request(openapi::InstanceFormat::INSTANCE_FORMAT_YAML);

        let expected = CreateParams {
            component: "arm-left".into(),
            role: None,
            aspect: None,
            serial_number: Some("SN-1".into()),
            reported_by: Some("Ada Lovelace".into()),
            format: InstanceFormat::Yaml,
            data: "offset: 1\n".into(),
        };
        assert_eq!(CreateParams::try_from(request).unwrap(), expected);
    }

    #[test]
    fn maps_each_known_format() {
        let cases = [
            (
                openapi::InstanceFormat::INSTANCE_FORMAT_JSON,
                InstanceFormat::Json,
            ),
            (
                openapi::InstanceFormat::INSTANCE_FORMAT_YAML,
                InstanceFormat::Yaml,
            ),
            (
                openapi::InstanceFormat::INSTANCE_FORMAT_JSONC,
                InstanceFormat::Jsonc,
            ),
            (
                openapi::InstanceFormat::INSTANCE_FORMAT_XML,
                InstanceFormat::Xml,
            ),
            (
                openapi::InstanceFormat::INSTANCE_FORMAT_CSV,
                InstanceFormat::Csv,
            ),
            (
                openapi::InstanceFormat::INSTANCE_FORMAT_TEXT,
                InstanceFormat::Text,
            ),
        ];
        for (format, want) in cases {
            let params = CreateParams::try_from(calibration_request(format)).unwrap();
            assert_eq!(params.format, want, "{format:?}");
        }
    }

    #[test]
    fn rejects_an_unknown_format() {
        let request = calibration_request(openapi::InstanceFormat::InstanceFormatUnknown);

        assert!(CreateParams::try_from(request).is_err());
    }
}

pub mod create_calibration_body {
    use super::*;

    fn body() -> CreateCalibrationBody {
        CreateCalibrationBody {
            request: calibration_request(openapi::InstanceFormat::INSTANCE_FORMAT_YAML),
            role: None,
            aspect: None,
        }
    }

    #[test]
    fn converts_the_request() {
        let request = calibration_request(openapi::InstanceFormat::INSTANCE_FORMAT_YAML);

        let expected = CreateParams::try_from(request).unwrap();
        assert_eq!(CreateParams::try_from(body()).unwrap(), expected);
    }

    #[test]
    fn rejects_a_role_or_aspect() {
        let role = CreateCalibrationBody {
            role: Some("left".into()),
            ..body()
        };
        let aspect = CreateCalibrationBody {
            aspect: Some("pose".into()),
            ..body()
        };

        assert!(CreateParams::try_from(role).is_err());
        assert!(CreateParams::try_from(aspect).is_err());
    }
}

pub mod latest_calibrations_query {
    use super::*;

    #[test]
    fn splits_comma_separated_values() {
        let query = LatestCalibrationsQuery {
            component: Some("arm, leg,,".into()),
            ..LatestCalibrationsQuery::default()
        };

        let expected = Filter {
            key: vec!["arm".into(), "leg".into()],
            ..Filter::default()
        };
        assert_eq!(Filter::try_from(query).unwrap(), expected);
    }

    #[test]
    fn an_empty_query_filters_nothing() {
        let filter = Filter::try_from(LatestCalibrationsQuery::default()).unwrap();

        assert_eq!(filter, Filter::default());
    }

    #[test]
    fn rejects_a_role_or_aspect() {
        let role = LatestCalibrationsQuery {
            role: Some("left".into()),
            ..LatestCalibrationsQuery::default()
        };
        let aspect = LatestCalibrationsQuery {
            aspect: Some("pose".into()),
            ..LatestCalibrationsQuery::default()
        };

        assert!(Filter::try_from(role).is_err());
        assert!(Filter::try_from(aspect).is_err());
    }
}
