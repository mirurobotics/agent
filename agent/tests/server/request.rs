// internal crates
use device_api::models as openapi;
use miru_agent::models::InstanceFormat;
use miru_agent::services::calibration::CreateParams;

fn calibration_request(format: openapi::InstanceFormat) -> openapi::CreateCalibrationRequest {
    openapi::CreateCalibrationRequest {
        component: "arm".into(),
        role: Some("left".into()),
        serial_number: Some("SN-1".into()),
        reported_by: Some("Ada Lovelace".into()),
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
            reported_by: Some("Ada Lovelace".into()),
            format: InstanceFormat::Yaml,
            data: "offset: 1\n".into(),
        };
        assert_eq!(CreateParams::try_from(request).unwrap(), expected);
    }

    #[test]
    fn accepts_a_missing_role_and_aspect() {
        let mut request = calibration_request(openapi::InstanceFormat::INSTANCE_FORMAT_JSON);
        request.role = None;
        request.aspect = None;

        let params = CreateParams::try_from(request).unwrap();
        assert_eq!((params.role, params.aspect), (None, None));
    }

    #[test]
    fn rejects_an_unknown_format() {
        let request = calibration_request(openapi::InstanceFormat::InstanceFormatUnknown);

        assert!(CreateParams::try_from(request).is_err());
    }

    #[test]
    fn rejects_a_null_role_or_aspect() {
        let mut role = calibration_request(openapi::InstanceFormat::INSTANCE_FORMAT_JSON);
        role.role = Some("null".into());
        let mut aspect = calibration_request(openapi::InstanceFormat::INSTANCE_FORMAT_JSON);
        aspect.aspect = Some("null".into());

        assert!(CreateParams::try_from(role).is_err());
        assert!(CreateParams::try_from(aspect).is_err());
    }
}
