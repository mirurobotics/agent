// internal crates
use device_api::models as openapi;
use miru_agent::models::InstanceFormat;
use miru_agent::services::calibration::CreateParams;

fn calibration_request(format: openapi::InstanceFormat) -> openapi::CreateCalibrationRequest {
    openapi::CreateCalibrationRequest {
        component_slug: "arm".into(),
        role: Some("left".into()),
        serial_number: Some("SN-1".into()),
        aspect: Some("pose".into()),
        content: Box::new(openapi::InstanceContent::new(format, "offset: 1\n".into())),
    }
}

pub mod create_calibration_request {
    use super::*;

    #[test]
    fn converts_every_field() {
        let request = calibration_request(openapi::InstanceFormat::INSTANCE_FORMAT_YAML);

        let expected = CreateParams {
            component: "arm".into(),
            role: Some("left".into()),
            aspect: Some("pose".into()),
            serial_number: Some("SN-1".into()),
            format: InstanceFormat::Yaml,
            data: "offset: 1\n".into(),
        };
        assert_eq!(CreateParams::try_from(request).unwrap(), expected);
    }

    #[test]
    fn rejects_an_unknown_format() {
        let request = calibration_request(openapi::InstanceFormat::InstanceFormatUnknown);

        assert!(CreateParams::try_from(request).is_err());
    }
}
