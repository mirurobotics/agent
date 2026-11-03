// internal crates
use crate::calibrations::{CalibrationNotFoundErr, CalibrationsErr, LatestContent, Uploader};
use crate::services::errors::ServiceErr;
use crate::trace;

// external crates
use uuid::Uuid;

/// The content of the latest calibration with `report_id`. An id that is not a
/// UUID is not found, like any other id that is not a latest entry.
pub async fn get_content(
    uploader: &Uploader,
    report_id: &str,
) -> Result<LatestContent, ServiceErr> {
    let Ok(id) = Uuid::parse_str(report_id) else {
        let err = CalibrationNotFoundErr {
            report_id: report_id.to_string(),
            trace: trace!(),
        };
        return Err(CalibrationsErr::from(err).into());
    };
    Ok(uploader.latest_content(id).await?)
}
