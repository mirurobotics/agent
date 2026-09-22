// internal crates
use crate::events;
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
            file_rule_ids: release.file_rule_ids.clone(),
            created_at: release.created_at.to_rfc3339(),
        }
    }
}

impl From<&models::FileRuleSource> for device_server::FileRuleSource {
    fn from(source: &models::FileRuleSource) -> Self {
        device_server::FileRuleSource {
            glob: source.glob.clone(),
            stability_window_secs: source.stability_window_secs,
        }
    }
}

impl From<&models::FileRuleUpload> for device_server::FileRuleUpload {
    fn from(upload: &models::FileRuleUpload) -> Self {
        device_server::FileRuleUpload {
            upload_collection_id: upload.upload_collection_id.clone(),
            upload_collection_name: upload.upload_collection_name.clone(),
            bucket_id: upload.bucket_id.clone(),
            bucket_name: upload.bucket_name.clone(),
            path: upload.path.clone(),
        }
    }
}

// The spec sends `require_upload` exactly when the rule has an `upload` block.
fn to_retention(
    retention: &models::FileRuleRetention,
    has_upload: bool,
) -> device_server::FileRuleRetention {
    device_server::FileRuleRetention {
        require_upload: has_upload.then_some(retention.require_upload),
        ttl_secs: i64::try_from(retention.ttl_secs).unwrap_or(i64::MAX),
    }
}

impl From<&models::FileRule> for device_server::BaseFileRule {
    fn from(rule: &models::FileRule) -> Self {
        let has_upload = rule.upload.is_some();
        device_server::BaseFileRule {
            object: device_server::base_file_rule::Object::FileRule,
            id: rule.id.clone(),
            name: rule.name.clone(),
            digest: rule.digest.clone(),
            source: Box::new((&rule.source).into()),
            upload: rule.upload.as_ref().map(|u| Box::new(u.into())),
            retention: rule
                .retention
                .as_ref()
                .map(|r| Box::new(to_retention(r, has_upload))),
            created_at: rule.created_at.to_rfc3339(),
            updated_at: rule.updated_at.to_rfc3339(),
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

impl From<&events::model::Event> for device_server::Event {
    fn from(event: &events::model::Event) -> Self {
        device_server::Event {
            object: device_server::event::Object::Event,
            id: event.id,
            r#type: event.event_type.clone(),
            occurred_at: event.occurred_at.to_rfc3339(),
            data: event.data.clone(),
        }
    }
}
