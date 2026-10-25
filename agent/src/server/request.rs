// internal crates
use crate::models::InstanceFormat;
use crate::server::errors::InvalidRequestErr;
use crate::services::calibration;
use crate::trace;
use device_api::models as device_server;

/// Checks the request's shape only; Miru judges the rest.
impl TryFrom<device_server::CreateCalibrationRequest> for calibration::CreateParams {
    type Error = InvalidRequestErr;

    fn try_from(request: device_server::CreateCalibrationRequest) -> Result<Self, Self::Error> {
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
            format,
            data: content.data,
        })
    }
}
