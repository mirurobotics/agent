// internal crates
use crate::models::InstanceFormat;
use crate::server::errors::InvalidRequestErr;
use crate::services::calibration;
use crate::trace;
use device_api::models as device_server;

/// Query keys the cloud calibration list forwards to Miru.
pub const CLOUD_CALIBRATION_LIST_KEYS: &[&str] = &[
    "offset",
    "limit",
    "order_by",
    "expand",
    "component",
    "role",
    "aspect",
    "serial_number",
    "report_id",
];

/// Query keys the cloud calibration get forwards to Miru.
pub const CLOUD_CALIBRATION_GET_KEYS: &[&str] = &["expand"];

/// Keeps a /cloud route's query for Miru, which validates the values. An
/// unknown key is rejected so a misspelled filter doesn't match everything.
pub fn forward_query(
    query: Vec<(String, String)>,
    keys: &[&'static str],
) -> Result<Vec<(&'static str, String)>, InvalidRequestErr> {
    query
        .into_iter()
        .map(|(name, value)| match keys.iter().find(|key| **key == name) {
            Some(key) => Ok((*key, value)),
            None => Err(InvalidRequestErr {
                msg: format!("unknown query parameter '{name}'"),
                trace: trace!(),
            }),
        })
        .collect()
}

/// Checks the request's shape and the reserved `null` slug, which the slug
/// filters use to match a missing role or aspect; Miru judges the rest.
impl TryFrom<device_server::CreateCalibrationRequest> for calibration::CreateParams {
    type Error = InvalidRequestErr;

    fn try_from(request: device_server::CreateCalibrationRequest) -> Result<Self, Self::Error> {
        for (field, slug) in [("role", &request.role), ("aspect", &request.aspect)] {
            if slug.as_deref() == Some("null") {
                return Err(InvalidRequestErr {
                    msg: format!("{field} 'null' is reserved"),
                    trace: trace!(),
                });
            }
        }
        let content = *request.content;
        let Some(format) = InstanceFormat::from_device_api(content.format) else {
            return Err(InvalidRequestErr {
                msg: "content.format is not a known format".to_string(),
                trace: trace!(),
            });
        };
        Ok(calibration::CreateParams {
            component: request.component,
            role: request.role,
            aspect: request.aspect,
            serial_number: request.serial_number,
            reported_by: request.reported_by,
            format,
            data: content.data,
        })
    }
}
