// internal crates
use crate::models;
use device_api::models as device_server;

impl From<&models::Device> for device_server::Device {
    fn from(device: &models::Device) -> Self {
        device_server::Device {
            object: device_server::device::Object::Device,
            id: device.id.clone(),
            name: device.name.clone(),
            status: (&device.status).into(),
            last_synced_at: device.last_synced_at.to_rfc3339(),
            last_connected_at: device.last_connected_at.to_rfc3339(),
            last_disconnected_at: device.last_disconnected_at.to_rfc3339(),
        }
    }
}

impl From<&models::Deployment> for device_server::Deployment {
    fn from(dpl: &models::Deployment) -> Self {
        let status = dpl.status();
        device_server::Deployment {
            object: device_server::deployment::Object::Deployment,
            id: dpl.id.clone(),
            description: dpl.description.clone(),
            status: (&status).into(),
            activity_status: (&dpl.activity_status).into(),
            error_status: (&dpl.error_status).into(),
            target_status: (&dpl.target_status).into(),
            device_id: dpl.device_id.clone(),
            release_id: dpl.release_id.clone(),
            created_at: dpl.created_at.to_rfc3339(),
        }
    }
}

impl From<&models::Release> for device_server::Release {
    fn from(release: &models::Release) -> Self {
        device_server::Release {
            object: device_server::release::Object::Release,
            id: release.id.clone(),
            version: release.version.clone(),
            git_commit_id: release.git_commit_id.clone(),
            created_at: release.created_at.to_rfc3339(),
        }
    }
}

impl From<&models::GitCommit> for device_server::GitCommit {
    fn from(gc: &models::GitCommit) -> Self {
        device_server::GitCommit {
            object: device_server::git_commit::Object::GitCommit,
            id: gc.id.clone(),
            sha: gc.sha.clone(),
            message: gc.message.clone(),
            commit_url: gc.commit_url.clone(),
            created_at: gc.created_at.to_rfc3339(),
        }
    }
}
