// internal crates
use crate::models::harnesses::assert_converts_each;
use device_api_v03::models as openapi;
use miru_daemon::models::{
    DeviceStatus, DplActivity, DplErrStatus, DplStatus, DplTarget, Os, UploadTimezone,
};
use miru_daemon::services::device::SyncCode;

#[test]
fn converts_each_device_status() {
    assert_converts_each(
        DeviceStatus::variants(),
        &[
            (
                DeviceStatus::Online,
                openapi::DeviceStatus::DEVICE_STATUS_ONLINE,
            ),
            (
                DeviceStatus::Offline,
                openapi::DeviceStatus::DEVICE_STATUS_OFFLINE,
            ),
        ],
    );
}

#[test]
fn converts_each_sync_code() {
    assert_converts_each(
        SyncCode::variants(),
        &[
            (SyncCode::Success, openapi::SyncResult::SYNC_RESULT_SUCCESS),
            (
                SyncCode::InCooldown,
                openapi::SyncResult::SYNC_RESULT_IN_COOLDOWN,
            ),
            (
                SyncCode::NetworkConnectionError,
                openapi::SyncResult::SYNC_RESULT_NETWORK_CONNECTION_ERROR,
            ),
        ],
    );
}

#[test]
fn converts_each_deployment_target_status() {
    assert_converts_each(
        DplTarget::variants(),
        &[
            (
                DplTarget::Staged,
                openapi::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_STAGED,
            ),
            (
                DplTarget::Deployed,
                openapi::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_DEPLOYED,
            ),
            (
                DplTarget::Archived,
                openapi::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_ARCHIVED,
            ),
        ],
    );
}

#[test]
fn converts_each_deployment_activity_status() {
    assert_converts_each(
        DplActivity::variants(),
        &[
            (
                DplActivity::Drifted,
                openapi::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DRIFTED,
            ),
            (
                DplActivity::Staged,
                openapi::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_STAGED,
            ),
            (
                DplActivity::Queued,
                openapi::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_QUEUED,
            ),
            (
                DplActivity::Deployed,
                openapi::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DEPLOYED,
            ),
            (
                DplActivity::Removing,
                openapi::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_REMOVING,
            ),
            (
                DplActivity::Archived,
                openapi::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_ARCHIVED,
            ),
        ],
    );
}

#[test]
fn converts_each_deployment_error_status() {
    assert_converts_each(
        DplErrStatus::variants(),
        &[
            (
                DplErrStatus::None,
                openapi::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_NONE,
            ),
            (
                DplErrStatus::Failed,
                openapi::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_FAILED,
            ),
            (
                DplErrStatus::Retrying,
                openapi::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_RETRYING,
            ),
        ],
    );
}

#[test]
fn converts_each_deployment_status() {
    assert_converts_each(
        DplStatus::variants(),
        &[
            (
                DplStatus::Drifted,
                openapi::DeploymentStatus::DEPLOYMENT_STATUS_DRIFTED,
            ),
            (
                DplStatus::Staged,
                openapi::DeploymentStatus::DEPLOYMENT_STATUS_STAGED,
            ),
            (
                DplStatus::Queued,
                openapi::DeploymentStatus::DEPLOYMENT_STATUS_QUEUED,
            ),
            (
                DplStatus::Deployed,
                openapi::DeploymentStatus::DEPLOYMENT_STATUS_DEPLOYED,
            ),
            (
                DplStatus::Removing,
                openapi::DeploymentStatus::DEPLOYMENT_STATUS_REMOVING,
            ),
            (
                DplStatus::Archived,
                openapi::DeploymentStatus::DEPLOYMENT_STATUS_ARCHIVED,
            ),
            (
                DplStatus::Failed,
                openapi::DeploymentStatus::DEPLOYMENT_STATUS_FAILED,
            ),
            (
                DplStatus::Retrying,
                openapi::DeploymentStatus::DEPLOYMENT_STATUS_RETRYING,
            ),
        ],
    );
}

#[test]
fn converts_each_os() {
    assert_converts_each(
        Os::variants(),
        &[
            (Os::Linux, openapi::Os::OS_LINUX),
            (Os::Windows, openapi::Os::OS_WINDOWS),
        ],
    );
}

#[test]
fn converts_each_upload_timezone() {
    assert_converts_each(
        UploadTimezone::variants(),
        &[
            (
                UploadTimezone::Utc,
                openapi::FileRuleUploadTimezone::FILE_RULE_UPLOAD_TIMEZONE_UTC,
            ),
            (
                UploadTimezone::Device,
                openapi::FileRuleUploadTimezone::FILE_RULE_UPLOAD_TIMEZONE_DEVICE,
            ),
        ],
    );
}
