// internal crates
use crate::calibrations::{CalibrationNotFoundErr, CalibrationsErr, LatestStore, Report};
use crate::services::errors::ServiceErr;
use crate::trace;

// external crates
use uuid::Uuid;

/// The latest calibration with `report_id`, with its data. An id that is not a
/// UUID is not found, like any other id that is not a latest entry.
pub async fn get_content(store: &LatestStore, report_id: &str) -> Result<Report, ServiceErr> {
    let Ok(id) = Uuid::parse_str(report_id) else {
        let err = CalibrationNotFoundErr {
            report_id: report_id.to_string(),
            trace: trace!(),
        };
        return Err(CalibrationsErr::from(err).into());
    };
    Ok(store.get(id).await?)
}
