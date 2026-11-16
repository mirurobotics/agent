// internal crates
use crate::calibrations::Filter;
use crate::models::InstanceFormat;
use crate::server::{errors::InvalidRequestErr, request::split_list};
use crate::services::calibration;
use crate::trace;
use device_api_v03::models as device_server;

// external crates
use serde::Deserialize;

/// The create body: the v0.3 request, and the `role` and `aspect` that v0.2
/// took, read only to reject them.
#[derive(Debug, Deserialize)]
pub struct CreateCalibrationBody {
    #[serde(flatten)]
    pub request: device_server::CreateCalibrationRequest,
    pub role: Option<serde_json::Value>,
    pub aspect: Option<serde_json::Value>,
}

impl TryFrom<CreateCalibrationBody> for calibration::CreateParams {
    type Error = InvalidRequestErr;

    fn try_from(body: CreateCalibrationBody) -> Result<Self, Self::Error> {
        reject_v02_fields(body.role.is_some(), body.aspect.is_some())?;
        calibration::CreateParams::try_from(body.request)
    }
}

/// Checks the request's shape; the control plane judges the rest.
impl TryFrom<device_server::CreateCalibrationRequest> for calibration::CreateParams {
    type Error = InvalidRequestErr;

    fn try_from(request: device_server::CreateCalibrationRequest) -> Result<Self, Self::Error> {
        let content = *request.content;
        let Some(format) = to_instance_format(content.format) else {
            return Err(InvalidRequestErr {
                msg: "content.format is not a known format".to_string(),
                trace: trace!(),
            });
        };
        Ok(calibration::CreateParams {
            component: request.component,
            role: None,
            aspect: None,
            serial_number: request.serial_number,
            reported_by: request.reported_by,
            format,
            data: content.data,
        })
    }
}

/// The format a request names, or `None` for one this agent doesn't know: a
/// request with it is rejected, not defaulted to text.
fn to_instance_format(format: device_server::InstanceFormat) -> Option<InstanceFormat> {
    match format {
        device_server::InstanceFormat::INSTANCE_FORMAT_JSON => Some(InstanceFormat::Json),
        device_server::InstanceFormat::INSTANCE_FORMAT_YAML => Some(InstanceFormat::Yaml),
        device_server::InstanceFormat::INSTANCE_FORMAT_JSONC => Some(InstanceFormat::Jsonc),
        device_server::InstanceFormat::INSTANCE_FORMAT_XML => Some(InstanceFormat::Xml),
        device_server::InstanceFormat::INSTANCE_FORMAT_CSV => Some(InstanceFormat::Csv),
        device_server::InstanceFormat::INSTANCE_FORMAT_TEXT => Some(InstanceFormat::Text),
        device_server::InstanceFormat::InstanceFormatUnknown => None,
    }
}

/// The `GET /calibrations/latest` query. The filter is a comma-separated
/// list, as the spec's `explode: false` sends it.
/// `role` and `aspect` are the v0.2 filters, read only to reject them.
#[derive(Debug, Default, Deserialize)]
pub struct LatestCalibrationsQuery {
    pub component: Option<String>,
    pub role: Option<String>,
    pub aspect: Option<String>,
}

impl TryFrom<LatestCalibrationsQuery> for Filter {
    type Error = InvalidRequestErr;

    fn try_from(query: LatestCalibrationsQuery) -> Result<Self, Self::Error> {
        reject_v02_fields(query.role.is_some(), query.aspect.is_some())?;
        Ok(Filter {
            key: split_list(query.component),
            ..Filter::default()
        })
    }
}

/// Rejects a `role` or `aspect`, which v0.3 removed, so a client that only
/// changed the version fails instead of having it ignored.
fn reject_v02_fields(has_role: bool, has_aspect: bool) -> Result<(), InvalidRequestErr> {
    for (field, is_set) in [("role", has_role), ("aspect", has_aspect)] {
        if is_set {
            return Err(InvalidRequestErr {
                msg: format!("{field} is not in v0.3; join it into component with '-'"),
                trace: trace!(),
            });
        }
    }
    Ok(())
}
