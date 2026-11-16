// internal crates
use device_api_v03::models as openapi;
use miru_daemon::calibrations::Filter;
use miru_daemon::models::InstanceFormat;
use miru_daemon::server::v03::request::LatestCalibrationsQuery;
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

pub mod latest_calibrations_query {
    use super::*;

    #[test]
    fn splits_comma_separated_values() {
        let query = LatestCalibrationsQuery {
            component: Some("arm, leg,,".into()),
        };

        let expected = Filter {
            flat_component: vec!["arm".into(), "leg".into()],
            ..Filter::default()
        };
        assert_eq!(Filter::from(query), expected);
    }

    #[test]
    fn an_empty_query_filters_nothing() {
        let filter = Filter::from(LatestCalibrationsQuery::default());

        assert_eq!(filter, Filter::default());
    }
}
