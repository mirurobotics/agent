// internal crates
use device_api::models as agent_server;
use miru_agent::models::CalibrationUploadStatus;

// harness
use crate::models::harnesses::{status_enum_tests, StatusCase, StatusEnumFixture};

impl StatusEnumFixture for CalibrationUploadStatus {
    type Backend = ();
    type Agent = agent_server::CalibrationUploadStatus;

    fn cases() -> Vec<StatusCase<Self>> {
        vec![
            StatusCase {
                variant: CalibrationUploadStatus::Queued,
                wire: "queued",
                backend: (),
                agent: agent_server::CalibrationUploadStatus::CALIBRATION_UPLOAD_STATUS_QUEUED,
            },
            StatusCase {
                variant: CalibrationUploadStatus::Uploaded,
                wire: "uploaded",
                backend: (),
                agent: agent_server::CalibrationUploadStatus::CALIBRATION_UPLOAD_STATUS_UPLOADED,
            },
            StatusCase {
                variant: CalibrationUploadStatus::Dropped,
                wire: "dropped",
                backend: (),
                agent: agent_server::CalibrationUploadStatus::CALIBRATION_UPLOAD_STATUS_DROPPED,
            },
        ]
    }
}

status_enum_tests!(CalibrationUploadStatus, agent);

#[test]
fn defaults_to_queued() {
    assert_eq!(
        CalibrationUploadStatus::default(),
        CalibrationUploadStatus::Queued
    );
}
