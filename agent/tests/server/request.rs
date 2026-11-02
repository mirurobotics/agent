// internal crates
use device_api::models as openapi;
use miru_agent::models::InstanceFormat;
use miru_agent::server::request::{
    forward_query, CLOUD_CALIBRATION_GET_KEYS, CLOUD_CALIBRATION_LIST_KEYS,
};
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

pub mod cloud_query {
    use super::*;

    fn pairs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn keeps_every_list_key_in_order() {
        let query = pairs(&[
            ("offset", "10"),
            ("limit", "5"),
            ("order_by", "reported_at:desc"),
            ("expand", "content"),
            ("expand", "total_count"),
            ("component", "arm,leg"),
            ("role", "null"),
            ("aspect", "pose"),
            ("serial_number", "SN-1"),
            ("report_id", "0b6f3c2e-6a51-4e0f-9a43-2f1d7c9e8b10"),
        ]);

        let forwarded = forward_query(query.clone(), CLOUD_CALIBRATION_LIST_KEYS).unwrap();

        let forwarded: Vec<(String, String)> = forwarded
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        assert_eq!(forwarded, query);
    }

    #[test]
    fn rejects_an_unknown_key() {
        let query = pairs(&[("expand", "content"), ("componet", "arm")]);

        assert!(forward_query(query, CLOUD_CALIBRATION_LIST_KEYS).is_err());
    }

    #[test]
    fn get_keys_allow_only_expand() {
        let expand = pairs(&[("expand", "content")]);
        let filter = pairs(&[("component", "arm")]);

        assert!(forward_query(expand, CLOUD_CALIBRATION_GET_KEYS).is_ok());
        assert!(forward_query(filter, CLOUD_CALIBRATION_GET_KEYS).is_err());
    }
}
