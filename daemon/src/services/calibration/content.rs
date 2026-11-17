// internal crates
use crate::calibrations::LatestStore;
use crate::models::InstanceFormat;
use crate::services::{calibration::parse_report_id, errors::ServiceErr};

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

/// The content of the latest calibration with `report_id`.
pub async fn get_content(store: &LatestStore, report_id: &str) -> Result<Content, ServiceErr> {
    let report = store.get(parse_report_id(report_id)?).await?;
    Ok(Content {
        report_id: report.metadata.report_id,
        format: report.metadata.format,
        data: report.data,
    })
}
