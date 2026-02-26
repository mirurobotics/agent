use crate::models;
use crate::models::deployment::{DplActivity, DplErrStatus, DplStatus, DplTarget};
use crate::models::device::DeviceStatus;
use openapi_server::models as openapi;

impl From<&models::Device> for openapi::Device {
    fn from(device: &models::Device) -> Self {
        openapi::Device {
            object: openapi::device::Object::Device,
            id: device.id.clone(),
            name: device.name.clone(),
            status: DeviceStatus::to_sdk(&device.status),
            last_synced_at: device.last_synced_at.to_rfc3339(),
            last_connected_at: device.last_connected_at.to_rfc3339(),
            last_disconnected_at: device.last_disconnected_at.to_rfc3339(),
        }
    }
}

impl From<&models::Deployment> for openapi::Deployment {
    fn from(dpl: &models::Deployment) -> Self {
        openapi::Deployment {
            object: openapi::deployment::Object::Deployment,
            id: dpl.id.clone(),
            description: dpl.description.clone(),
            status: DplStatus::to_sdk(&dpl.status()),
            activity_status: DplActivity::to_sdk(&dpl.activity_status),
            error_status: DplErrStatus::to_sdk(&dpl.error_status),
            target_status: DplTarget::to_sdk(&dpl.target_status),
            device_id: dpl.device_id.clone(),
            release_id: dpl.release_id.clone(),
            created_at: dpl.created_at.to_rfc3339(),
            updated_at: dpl.updated_at.to_rfc3339(),
            config_instances: None,
        }
    }
}

impl From<&models::Release> for openapi::Release {
    fn from(release: &models::Release) -> Self {
        openapi::Release {
            object: openapi::release::Object::Release,
            id: release.id.clone(),
            version: release.version.clone(),
            git_commit_id: release.git_commit_id.clone(),
            created_at: release.created_at.to_rfc3339(),
            updated_at: release.updated_at.to_rfc3339(),
        }
    }
}

impl From<&models::GitCommit> for openapi::GitCommit {
    fn from(gc: &models::GitCommit) -> Self {
        openapi::GitCommit {
            object: openapi::git_commit::Object::GitCommit,
            id: gc.id.clone(),
            sha: gc.sha.clone(),
            message: gc.message.clone(),
            commit_url: gc.commit_url.clone(),
            created_at: gc.created_at.to_rfc3339(),
        }
    }
}
