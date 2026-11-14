// internal crates
use crate::calibrations::{CalibrationNotFoundErr, CalibrationsErr, LatestStore};
use crate::models::InstanceFormat;
use crate::services::errors::ServiceErr;
use crate::trace;

// external crates
use uuid::Uuid;

/// A latest calibration's data, with what its content response needs.
#[derive(Clone, Debug, PartialEq)]
pub struct Content {
    pub report_id: Uuid,
    /// The format `data` is serialized in.
    pub format: InstanceFormat,
    pub data: String,
}

/// The content of the latest calibration with `report_id`. An id that is not a
/// UUID is not found, like any other id that is not a latest entry.
pub async fn get_content(store: &LatestStore, report_id: &str) -> Result<Content, ServiceErr> {
    let Ok(id) = Uuid::parse_str(report_id) else {
        let err = CalibrationNotFoundErr {
            report_id: report_id.to_string(),
            trace: trace!(),
        };
        return Err(CalibrationsErr::from(err).into());
    };
    let report = store.get(id).await?;
    Ok(Content {
        report_id: report.metadata.report_id,
        format: report.metadata.format,
        data: report.data,
    })
}
