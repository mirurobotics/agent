// internal crates
use crate::models::status::impl_status_enum;
use device_api::models as agent_server;

// external crates
use serde::Serialize;
use tracing::warn;

/// Whether the agent has uploaded a latest calibration to Miru.
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CalibrationUploadStatus {
    #[default]
    Queued,
    Uploaded,
    Dropped,
}

impl_status_enum!(
    enum CalibrationUploadStatus,
    default: Queued,
    label: "calibration upload status",
    log: warn,
    agent_type: agent_server::CalibrationUploadStatus,
    mappings: [
        Queued => "queued" =>
            agent_server::CalibrationUploadStatus::CALIBRATION_UPLOAD_STATUS_QUEUED,
        Uploaded => "uploaded" =>
            agent_server::CalibrationUploadStatus::CALIBRATION_UPLOAD_STATUS_UPLOADED,
        Dropped => "dropped" =>
            agent_server::CalibrationUploadStatus::CALIBRATION_UPLOAD_STATUS_DROPPED,
    ]
);
