use chrono::{DateTime, TimeZone, Utc};
use miru_agent::models::deployment::{DplActivity, DplErrStatus, DplTarget};
use miru_agent::models::device::DeviceStatus;
use miru_agent::models::{Deployment, Device, GitCommit, Release};
use openapi_server::models as openapi;

fn fixed_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2025, 6, 15, 12, 0, 0).unwrap()
}

pub mod device_response {
    use super::*;

    #[test]
    fn converts_offline_device() {
        let t = fixed_time();
        let device = Device {
            id: "dev-1".into(),
            session_id: "sess-1".into(),
            name: "robot-1".into(),
            agent_version: "1.0.0".into(),
            activated: false,
            status: DeviceStatus::Offline,
            last_synced_at: t,
            last_connected_at: t,
            last_disconnected_at: t,
        };

        let expected = openapi::Device {
            object: openapi::device::Object::Device,
            id: "dev-1".into(),
            name: "robot-1".into(),
            status: openapi::DeviceStatus::DEVICE_STATUS_OFFLINE,
            last_synced_at: t.to_rfc3339(),
            last_connected_at: t.to_rfc3339(),
            last_disconnected_at: t.to_rfc3339(),
        };

        let sdk: openapi::Device = (&device).into();
        assert_eq!(sdk, expected);
    }

    #[test]
    fn converts_online_device() {
        let t = fixed_time();
        let device = Device {
            id: "dev-2".into(),
            session_id: "sess-2".into(),
            name: "robot-2".into(),
            agent_version: "2.0.0".into(),
            activated: true,
            status: DeviceStatus::Online,
            last_synced_at: t,
            last_connected_at: t,
            last_disconnected_at: t,
        };

        let expected = openapi::Device {
            object: openapi::device::Object::Device,
            id: "dev-2".into(),
            name: "robot-2".into(),
            status: openapi::DeviceStatus::DEVICE_STATUS_ONLINE,
            last_synced_at: t.to_rfc3339(),
            last_connected_at: t.to_rfc3339(),
            last_disconnected_at: t.to_rfc3339(),
        };

        let sdk: openapi::Device = (&device).into();
        assert_eq!(sdk, expected);
    }
}

pub mod deployment_response {
    use super::*;

    #[test]
    fn converts_staged_deployment() {
        let t = fixed_time();
        let dpl = Deployment {
            id: "dpl-1".into(),
            description: "staging deploy".into(),
            activity_status: DplActivity::Staged,
            error_status: DplErrStatus::None,
            target_status: DplTarget::Staged,
            device_id: "dev-1".into(),
            release_id: "rls-1".into(),
            created_at: t,
            updated_at: t,
            ..Default::default()
        };

        let expected = openapi::Deployment {
            object: openapi::deployment::Object::Deployment,
            id: "dpl-1".into(),
            description: "staging deploy".into(),
            status: openapi::DeploymentStatus::DEPLOYMENT_STATUS_STAGED,
            activity_status: openapi::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_STAGED,
            error_status: openapi::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_NONE,
            target_status: openapi::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_STAGED,
            device_id: "dev-1".into(),
            release_id: "rls-1".into(),
            created_at: t.to_rfc3339(),
            updated_at: t.to_rfc3339(),
            config_instances: None,
        };

        let sdk: openapi::Deployment = (&dpl).into();
        assert_eq!(sdk, expected);
    }

    #[test]
    fn converts_deployed_deployment() {
        let t = fixed_time();
        let dpl = Deployment {
            id: "dpl-2".into(),
            description: "production deploy".into(),
            activity_status: DplActivity::Deployed,
            error_status: DplErrStatus::None,
            target_status: DplTarget::Deployed,
            device_id: "dev-1".into(),
            release_id: "rls-1".into(),
            created_at: t,
            updated_at: t,
            ..Default::default()
        };

        let expected = openapi::Deployment {
            object: openapi::deployment::Object::Deployment,
            id: "dpl-2".into(),
            description: "production deploy".into(),
            status: openapi::DeploymentStatus::DEPLOYMENT_STATUS_DEPLOYED,
            activity_status: openapi::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DEPLOYED,
            error_status: openapi::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_NONE,
            target_status: openapi::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_DEPLOYED,
            device_id: "dev-1".into(),
            release_id: "rls-1".into(),
            created_at: t.to_rfc3339(),
            updated_at: t.to_rfc3339(),
            config_instances: None,
        };

        let sdk: openapi::Deployment = (&dpl).into();
        assert_eq!(sdk, expected);
    }

    #[test]
    fn converts_failed_deployment() {
        let t = fixed_time();
        let dpl = Deployment {
            id: "dpl-3".into(),
            description: "broken deploy".into(),
            activity_status: DplActivity::Queued,
            error_status: DplErrStatus::Failed,
            target_status: DplTarget::Deployed,
            device_id: "dev-1".into(),
            release_id: "rls-1".into(),
            created_at: t,
            updated_at: t,
            ..Default::default()
        };

        let expected = openapi::Deployment {
            object: openapi::deployment::Object::Deployment,
            id: "dpl-3".into(),
            description: "broken deploy".into(),
            status: openapi::DeploymentStatus::DEPLOYMENT_STATUS_FAILED,
            activity_status: openapi::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_QUEUED,
            error_status: openapi::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_FAILED,
            target_status: openapi::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_DEPLOYED,
            device_id: "dev-1".into(),
            release_id: "rls-1".into(),
            created_at: t.to_rfc3339(),
            updated_at: t.to_rfc3339(),
            config_instances: None,
        };

        let sdk: openapi::Deployment = (&dpl).into();
        assert_eq!(sdk, expected);
    }

    #[test]
    fn converts_retrying_deployment() {
        let t = fixed_time();
        let dpl = Deployment {
            id: "dpl-4".into(),
            description: "retrying deploy".into(),
            activity_status: DplActivity::Queued,
            error_status: DplErrStatus::Retrying,
            target_status: DplTarget::Deployed,
            device_id: "dev-1".into(),
            release_id: "rls-1".into(),
            created_at: t,
            updated_at: t,
            ..Default::default()
        };

        let expected = openapi::Deployment {
            object: openapi::deployment::Object::Deployment,
            id: "dpl-4".into(),
            description: "retrying deploy".into(),
            status: openapi::DeploymentStatus::DEPLOYMENT_STATUS_RETRYING,
            activity_status: openapi::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_QUEUED,
            error_status: openapi::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_RETRYING,
            target_status: openapi::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_DEPLOYED,
            device_id: "dev-1".into(),
            release_id: "rls-1".into(),
            created_at: t.to_rfc3339(),
            updated_at: t.to_rfc3339(),
            config_instances: None,
        };

        let sdk: openapi::Deployment = (&dpl).into();
        assert_eq!(sdk, expected);
    }
}

pub mod release_response {
    use super::*;

    #[test]
    fn converts_release_without_git_commit() {
        let t = fixed_time();
        let rls = Release {
            id: "rls-1".into(),
            version: "1.0.0".into(),
            git_commit_id: None,
            created_at: t,
            updated_at: t,
        };

        let expected = openapi::Release {
            object: openapi::release::Object::Release,
            id: "rls-1".into(),
            version: "1.0.0".into(),
            git_commit_id: None,
            created_at: t.to_rfc3339(),
            updated_at: t.to_rfc3339(),
        };

        let sdk: openapi::Release = (&rls).into();
        assert_eq!(sdk, expected);
    }

    #[test]
    fn converts_release_with_git_commit() {
        let t = fixed_time();
        let rls = Release {
            id: "rls-2".into(),
            version: "2.0.0".into(),
            git_commit_id: Some("gc-1".into()),
            created_at: t,
            updated_at: t,
        };

        let expected = openapi::Release {
            object: openapi::release::Object::Release,
            id: "rls-2".into(),
            version: "2.0.0".into(),
            git_commit_id: Some("gc-1".into()),
            created_at: t.to_rfc3339(),
            updated_at: t.to_rfc3339(),
        };

        let sdk: openapi::Release = (&rls).into();
        assert_eq!(sdk, expected);
    }
}

pub mod git_commit_response {
    use super::*;

    #[test]
    fn converts_git_commit() {
        let t = fixed_time();
        let gc = GitCommit {
            id: "gc-1".into(),
            sha: "abc123def456".into(),
            message: "feat: add new feature".into(),
            repository_owner: "owner".into(),
            repository_name: "repo".into(),
            repository_type: "github".into(),
            repository_url: "https://github.com/owner/repo".into(),
            commit_url: "https://github.com/owner/repo/commit/abc123".into(),
            created_at: t,
        };

        let expected = openapi::GitCommit {
            object: openapi::git_commit::Object::GitCommit,
            id: "gc-1".into(),
            sha: "abc123def456".into(),
            message: "feat: add new feature".into(),
            commit_url: "https://github.com/owner/repo/commit/abc123".into(),
            created_at: t.to_rfc3339(),
        };

        let sdk: openapi::GitCommit = (&gc).into();
        assert_eq!(sdk, expected);
    }
}
