// internal crates
use crate::calibrations::Filter;
use crate::models::InstanceFormat;
use crate::server::errors::InvalidRequestErr;
use crate::services::calibration;
use crate::trace;
use device_api_v03::models as device_server;

// external crates
use serde::Deserialize;

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
#[derive(Debug, Default, Deserialize)]
pub struct LatestCalibrationsQuery {
    pub component: Option<String>,
}

impl From<LatestCalibrationsQuery> for Filter {
    fn from(query: LatestCalibrationsQuery) -> Self {
        Filter {
            flat_component: split_list(query.component),
            ..Filter::default()
        }
    }
}

/// The trimmed, non-empty items of a comma-separated list.
pub(crate) fn split_list(list: Option<String>) -> Vec<String> {
    let list = list.unwrap_or_default();
    let items = list.split(',').map(|item| item.trim().to_string());
    items.filter(|item| !item.is_empty()).collect()
}
