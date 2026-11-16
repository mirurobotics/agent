// internal crates
use crate::models;
use crate::services::device::SyncCode;
use device_api_v03::models as device_server;

impl From<&models::DeviceStatus> for device_server::DeviceStatus {
    fn from(status: &models::DeviceStatus) -> Self {
        match status {
            models::DeviceStatus::Online => Self::DEVICE_STATUS_ONLINE,
            models::DeviceStatus::Offline => Self::DEVICE_STATUS_OFFLINE,
        }
    }
}

impl From<&SyncCode> for device_server::SyncDeviceResult {
    fn from(code: &SyncCode) -> Self {
        match code {
            SyncCode::Success => Self::SYNC_DEVICE_RESULT_SUCCESS,
            SyncCode::InCooldown => Self::SYNC_DEVICE_RESULT_IN_COOLDOWN,
            SyncCode::NetworkConnectionError => Self::SYNC_DEVICE_RESULT_NETWORK_CONNECTION_ERROR,
        }
    }
}

impl From<&models::DplStatus> for device_server::DeploymentStatus {
    fn from(status: &models::DplStatus) -> Self {
        match status {
            models::DplStatus::Drifted => Self::DEPLOYMENT_STATUS_DRIFTED,
            models::DplStatus::Staged => Self::DEPLOYMENT_STATUS_STAGED,
            models::DplStatus::Queued => Self::DEPLOYMENT_STATUS_QUEUED,
            models::DplStatus::Deployed => Self::DEPLOYMENT_STATUS_DEPLOYED,
            models::DplStatus::Removing => Self::DEPLOYMENT_STATUS_REMOVING,
            models::DplStatus::Archived => Self::DEPLOYMENT_STATUS_ARCHIVED,
            models::DplStatus::Failed => Self::DEPLOYMENT_STATUS_FAILED,
            models::DplStatus::Retrying => Self::DEPLOYMENT_STATUS_RETRYING,
        }
    }
}

impl From<&models::DplActivity> for device_server::DeploymentActivityStatus {
    fn from(status: &models::DplActivity) -> Self {
        match status {
            models::DplActivity::Drifted => Self::DEPLOYMENT_ACTIVITY_STATUS_DRIFTED,
            models::DplActivity::Staged => Self::DEPLOYMENT_ACTIVITY_STATUS_STAGED,
            models::DplActivity::Queued => Self::DEPLOYMENT_ACTIVITY_STATUS_QUEUED,
            models::DplActivity::Deployed => Self::DEPLOYMENT_ACTIVITY_STATUS_DEPLOYED,
            models::DplActivity::Removing => Self::DEPLOYMENT_ACTIVITY_STATUS_REMOVING,
            models::DplActivity::Archived => Self::DEPLOYMENT_ACTIVITY_STATUS_ARCHIVED,
        }
    }
}

impl From<&models::DplErrStatus> for device_server::DeploymentErrorStatus {
    fn from(status: &models::DplErrStatus) -> Self {
        match status {
            models::DplErrStatus::None => Self::DEPLOYMENT_ERROR_STATUS_NONE,
            models::DplErrStatus::Failed => Self::DEPLOYMENT_ERROR_STATUS_FAILED,
            models::DplErrStatus::Retrying => Self::DEPLOYMENT_ERROR_STATUS_RETRYING,
        }
    }
}

impl From<&models::DplTarget> for device_server::DeploymentTargetStatus {
    fn from(status: &models::DplTarget) -> Self {
        match status {
            models::DplTarget::Staged => Self::DEPLOYMENT_TARGET_STATUS_STAGED,
            models::DplTarget::Deployed => Self::DEPLOYMENT_TARGET_STATUS_DEPLOYED,
            models::DplTarget::Archived => Self::DEPLOYMENT_TARGET_STATUS_ARCHIVED,
        }
    }
}

impl From<&models::Os> for device_server::Os {
    fn from(os: &models::Os) -> Self {
        match os {
            models::Os::Linux => Self::OS_LINUX,
            models::Os::Windows => Self::OS_WINDOWS,
        }
    }
}

impl From<&models::UploadTimezone> for device_server::FileRuleUploadTimezone {
    fn from(timezone: &models::UploadTimezone) -> Self {
        match timezone {
            models::UploadTimezone::Utc => Self::FILE_RULE_UPLOAD_TIMEZONE_UTC,
            models::UploadTimezone::Device => Self::FILE_RULE_UPLOAD_TIMEZONE_DEVICE,
        }
    }
}
