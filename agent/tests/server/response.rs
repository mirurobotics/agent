// internal crates
use device_api::models as openapi;
use miru_agent::models::{
    Deployment, Device, DeviceStatus, DplActivity, DplErrStatus, DplTarget, FileRule,
    FileRuleRetention, FileRuleSource, FileRuleUpload, GitCommit, Release,
};

// external crates
use chrono::{DateTime, TimeZone, Utc};

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
            file_rule_ids: Vec::new(),
        };

        let expected = openapi::Release {
            object: openapi::release::Object::Release,
            id: "rls-1".into(),
            version: "1.0.0".into(),
            git_commit_id: None,
            file_rule_ids: Vec::new(),
            created_at: t.to_rfc3339(),
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
            file_rule_ids: Vec::new(),
        };

        let expected = openapi::Release {
            object: openapi::release::Object::Release,
            id: "rls-2".into(),
            version: "2.0.0".into(),
            git_commit_id: Some("gc-1".into()),
            file_rule_ids: Vec::new(),
            created_at: t.to_rfc3339(),
        };

        let sdk: openapi::Release = (&rls).into();
        assert_eq!(sdk, expected);
    }

    #[test]
    fn converts_release_with_file_rule_ids() {
        let t = fixed_time();
        let rls = Release {
            id: "rls-3".into(),
            version: "3.0.0".into(),
            git_commit_id: None,
            created_at: t,
            updated_at: t,
            file_rule_ids: vec!["fr-1".into(), "fr-2".into()],
        };

        let expected = openapi::Release {
            object: openapi::release::Object::Release,
            id: "rls-3".into(),
            version: "3.0.0".into(),
            git_commit_id: None,
            file_rule_ids: vec!["fr-1".into(), "fr-2".into()],
            created_at: t.to_rfc3339(),
        };

        let sdk: openapi::Release = (&rls).into();
        assert_eq!(sdk, expected);
    }
}

pub mod file_rule_response {
    use super::*;

    fn upload() -> FileRuleUpload {
        FileRuleUpload {
            upload_collection_id: "uc-1".into(),
            upload_collection_name: "logs".into(),
            bucket_id: "bkt-1".into(),
            bucket_name: "fleet-logs".into(),
            path: "robots/".into(),
        }
    }

    fn rule(upload: Option<FileRuleUpload>, retention: Option<FileRuleRetention>) -> FileRule {
        let t = fixed_time();
        FileRule {
            id: "fr-1".into(),
            name: "logs".into(),
            digest: "digest-1".into(),
            source: FileRuleSource {
                glob: "/var/log/app/*.log".into(),
                stability_window_secs: 30,
            },
            upload,
            retention,
            created_at: t,
            updated_at: t,
        }
    }

    #[test]
    fn converts_upload_rule_with_retention() {
        let t = fixed_time();
        let rule = rule(
            Some(upload()),
            Some(FileRuleRetention {
                require_upload: true,
                ttl_secs: 3600,
            }),
        );

        let expected = openapi::BaseFileRule {
            object: openapi::base_file_rule::Object::FileRule,
            id: "fr-1".into(),
            name: "logs".into(),
            digest: "digest-1".into(),
            source: Box::new(openapi::FileRuleSource {
                glob: "/var/log/app/*.log".into(),
                stability_window_secs: 30,
            }),
            upload: Some(Box::new(openapi::FileRuleUpload {
                upload_collection_id: "uc-1".into(),
                upload_collection_name: "logs".into(),
                bucket_id: "bkt-1".into(),
                bucket_name: "fleet-logs".into(),
                path: "robots/".into(),
            })),
            retention: Some(Box::new(openapi::FileRuleRetention {
                require_upload: Some(true),
                ttl_secs: 3600,
            })),
            created_at: t.to_rfc3339(),
            updated_at: t.to_rfc3339(),
        };

        let sdk: openapi::BaseFileRule = (&rule).into();
        assert_eq!(sdk, expected);
    }

    #[test]
    fn converts_upload_rule_with_best_effort_retention() {
        let rule = rule(
            Some(upload()),
            Some(FileRuleRetention {
                require_upload: false,
                ttl_secs: 60,
            }),
        );

        let expected = openapi::FileRuleRetention {
            require_upload: Some(false),
            ttl_secs: 60,
        };

        let sdk: openapi::BaseFileRule = (&rule).into();
        assert_eq!(sdk.retention, Some(Box::new(expected)));
    }

    #[test]
    fn converts_retention_only_rule_omits_require_upload() {
        // require_upload is dropped whenever the rule has no upload block,
        // whatever the stored value.
        let rule = rule(
            None,
            Some(FileRuleRetention {
                require_upload: true,
                ttl_secs: 0,
            }),
        );

        let expected = openapi::FileRuleRetention {
            require_upload: None,
            ttl_secs: 0,
        };

        let sdk: openapi::BaseFileRule = (&rule).into();
        assert!(sdk.upload.is_none());
        assert_eq!(sdk.retention, Some(Box::new(expected)));

        let json = serde_json::to_value(&sdk).unwrap();
        assert!(json.get("upload").is_none());
        assert!(json["retention"].get("require_upload").is_none());
    }

    #[test]
    fn converts_rule_without_retention() {
        let rule = rule(Some(upload()), None);

        let sdk: openapi::BaseFileRule = (&rule).into();
        assert!(sdk.retention.is_none());
        assert!(serde_json::to_value(&sdk)
            .unwrap()
            .get("retention")
            .is_none());
    }

    #[test]
    fn saturates_ttl_secs_above_i64_max() {
        let rule = rule(
            None,
            Some(FileRuleRetention {
                require_upload: false,
                ttl_secs: u64::MAX,
            }),
        );

        let sdk: openapi::BaseFileRule = (&rule).into();
        assert_eq!(sdk.retention.unwrap().ttl_secs, i64::MAX);
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
