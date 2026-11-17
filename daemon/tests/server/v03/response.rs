// internal crates
use device_api_v03::models as openapi;
use miru_daemon::models::{
    Deployment, Device, DeviceStatus, DplActivity, DplErrStatus, DplTarget, FileRule,
    FileRuleRetention, FileRuleSource, FileRuleUpload, GitCommit, Release, UploadTimezone,
};
use miru_daemon::services::calibration::Created;
use miru_daemon::services::device::{SyncCode, SyncOutcome};
use miru_daemon::sync::syncer::State;

// external crates
use chrono::{DateTime, Duration, TimeZone, Utc};
use uuid::Uuid;

fn fixed_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2025, 6, 15, 12, 0, 0).unwrap()
}

pub mod device_response {
    use super::*;
    use miru_daemon::server::v03::response;

    fn device_at(status: DeviceStatus, at: DateTime<Utc>) -> Device {
        Device {
            id: "dev-1".into(),
            session_id: "sess-1".into(),
            name: "robot-1".into(),
            activated: true,
            status,
            last_synced_at: at,
            last_connected_at: at,
            last_disconnected_at: at,
        }
    }

    fn device_times(device: &openapi::Device) -> (Option<i64>, Option<i64>, Option<i64>) {
        (
            device.last_synced_secs_ago,
            device.last_connected_secs_ago,
            device.last_disconnected_secs_ago,
        )
    }

    #[test]
    fn converts_offline_device() {
        let now = fixed_time();
        let device = Device {
            last_synced_at: now - Duration::seconds(10),
            last_connected_at: now - Duration::seconds(20),
            last_disconnected_at: now - Duration::seconds(30),
            ..device_at(DeviceStatus::Offline, now)
        };

        let expected = openapi::Device {
            object: openapi::device::Object::Device,
            id: "dev-1".into(),
            name: "robot-1".into(),
            status: openapi::DeviceStatus::DEVICE_STATUS_OFFLINE,
            last_synced_secs_ago: Some(10),
            last_connected_secs_ago: Some(20),
            last_disconnected_secs_ago: Some(30),
        };

        assert_eq!(response::device(&device, now), expected);
    }

    #[test]
    fn converts_online_device() {
        let now = fixed_time();
        let device = device_at(DeviceStatus::Online, now);

        let actual = response::device(&device, now);
        assert_eq!(actual.status, openapi::DeviceStatus::DEVICE_STATUS_ONLINE);
    }

    #[test]
    fn reports_null_for_times_never_set() {
        let device = device_at(DeviceStatus::Online, DateTime::<Utc>::UNIX_EPOCH);

        let actual = response::device(&device, fixed_time());
        assert_eq!(device_times(&actual), (None, None, None));
    }

    #[test]
    fn reports_a_time_from_an_earlier_run() {
        let device = device_at(DeviceStatus::Online, fixed_time() - Duration::days(2));

        let actual = response::device(&device, fixed_time());
        let two_days = Some(2 * 24 * 3600);
        assert_eq!(device_times(&actual), (two_days, two_days, two_days));
    }

    #[test]
    fn rounds_secs_ago_down() {
        let now = fixed_time();
        let at = now - Duration::milliseconds(90_999);
        let device = device_at(DeviceStatus::Online, at);

        let actual = response::device(&device, now);
        assert_eq!(device_times(&actual), (Some(90), Some(90), Some(90)));
    }

    #[test]
    fn reports_0_for_a_time_after_now() {
        let now = fixed_time();
        let device = device_at(DeviceStatus::Online, now + Duration::seconds(5));

        let actual = response::device(&device, now);
        assert_eq!(device_times(&actual), (Some(0), Some(0), Some(0)));
    }
}

pub mod sync_response {
    use super::*;
    use miru_daemon::server::v03::response;

    fn outcome(code: SyncCode, cooldown_ends_at: DateTime<Utc>) -> SyncOutcome {
        let now = fixed_time();
        SyncOutcome {
            code,
            message: "sync message".into(),
            state: State {
                last_attempted_sync_at: now - Duration::seconds(10),
                last_synced_at: now - Duration::minutes(5),
                cooldown_ends_at,
                err_streak: 0,
            },
        }
    }

    fn cooldown(body: &openapi::SyncResponse) -> (bool, i64) {
        (body.in_cooldown, body.cooldown_ends_in_secs)
    }

    #[test]
    fn converts_outcome_in_cooldown() {
        let now = fixed_time();
        let outcome = outcome(SyncCode::InCooldown, now + Duration::seconds(15));

        let expected = openapi::SyncResponse {
            code: openapi::SyncResult::SYNC_RESULT_IN_COOLDOWN,
            message: "sync message".into(),
            last_synced_secs_ago: Some(300),
            last_attempted_sync_secs_ago: 10,
            in_cooldown: true,
            cooldown_ends_in_secs: 15,
        };

        assert_eq!(response::sync(&outcome, now), expected);
    }

    #[test]
    fn converts_outcome_after_cooldown() {
        let now = fixed_time();
        let outcome = outcome(SyncCode::Success, now - Duration::seconds(1));

        let expected = openapi::SyncResponse {
            code: openapi::SyncResult::SYNC_RESULT_SUCCESS,
            message: "sync message".into(),
            last_synced_secs_ago: Some(300),
            last_attempted_sync_secs_ago: 10,
            in_cooldown: false,
            cooldown_ends_in_secs: 0,
        };

        assert_eq!(response::sync(&outcome, now), expected);
    }

    #[test]
    fn reports_null_last_synced_never_set() {
        let now = fixed_time();
        let mut outcome = outcome(SyncCode::Success, now);
        outcome.state.last_synced_at = DateTime::<Utc>::UNIX_EPOCH;

        let actual = response::sync(&outcome, now);
        assert_eq!(actual.last_synced_secs_ago, None);
    }

    #[test]
    fn rounds_cooldown_ends_in_secs_up() {
        let now = fixed_time();
        let outcome = outcome(SyncCode::Success, now + Duration::milliseconds(1_500));

        let actual = response::sync(&outcome, now);
        assert_eq!(cooldown(&actual), (true, 2));
    }

    #[test]
    fn is_in_cooldown_until_it_ends() {
        let now = fixed_time();
        let outcome = outcome(SyncCode::Success, now + Duration::milliseconds(1));

        let actual = response::sync(&outcome, now);
        assert_eq!(cooldown(&actual), (true, 1));
    }

    #[test]
    fn is_not_in_cooldown_when_it_ends_now() {
        let now = fixed_time();
        let outcome = outcome(SyncCode::Success, now);

        let actual = response::sync(&outcome, now);
        assert_eq!(cooldown(&actual), (false, 0));
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
            os: miru_daemon::models::Os::Linux,
            created_at: t,
            updated_at: t,
            file_rule_ids: Vec::new(),
        };

        let expected = openapi::Release {
            object: openapi::release::Object::Release,
            id: "rls-1".into(),
            version: "1.0.0".into(),
            git_commit_id: None,
            os: openapi::Os::OS_LINUX,
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
            os: miru_daemon::models::Os::Linux,
            created_at: t,
            updated_at: t,
            file_rule_ids: Vec::new(),
        };

        let expected = openapi::Release {
            object: openapi::release::Object::Release,
            id: "rls-2".into(),
            version: "2.0.0".into(),
            git_commit_id: Some("gc-1".into()),
            os: openapi::Os::OS_LINUX,
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
            os: miru_daemon::models::Os::Windows,
            created_at: t,
            updated_at: t,
            file_rule_ids: vec!["fr-1".into(), "fr-2".into()],
        };

        let expected = openapi::Release {
            object: openapi::release::Object::Release,
            id: "rls-3".into(),
            version: "3.0.0".into(),
            git_commit_id: None,
            os: openapi::Os::OS_WINDOWS,
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
            timezone: UploadTimezone::Device,
            bundle: Some(vec!["robot".into()]),
        }
    }

    fn rule(upload: Option<FileRuleUpload>, retention: Option<FileRuleRetention>) -> FileRule {
        let t = fixed_time();
        FileRule {
            id: "fr-1".into(),
            name: "logs".into(),
            digest: "digest-1".into(),
            os: miru_daemon::models::Os::Windows,
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
                require_upload: Some(true),
                ttl_secs: 3600,
            }),
        );

        let expected = openapi::BaseFileRule {
            object: openapi::base_file_rule::Object::FileRule,
            id: "fr-1".into(),
            name: "logs".into(),
            digest: "digest-1".into(),
            os: openapi::Os::OS_WINDOWS,
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
                timezone: openapi::FileRuleUploadTimezone::FILE_RULE_UPLOAD_TIMEZONE_DEVICE,
                bundle: Some(vec!["robot".into()]),
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
                require_upload: Some(false),
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
        // A stored absence is copied through and omitted from the JSON.
        let rule = rule(
            None,
            Some(FileRuleRetention {
                require_upload: None,
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
                require_upload: Some(false),
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

pub mod event_response {
    use super::*;
    use miru_daemon::events::errors::EventsErr;
    use miru_daemon::events::model::{Event, EventArgs};

    fn stored(id: i64, args: EventArgs) -> Event {
        Event {
            id,
            event_type: args.event_type,
            occurred_at: fixed_time(),
            data: args.data,
        }
    }

    fn deployment() -> Deployment {
        Deployment {
            id: "dpl-1".into(),
            release_id: "rls-1".into(),
            activity_status: DplActivity::Deployed,
            error_status: DplErrStatus::None,
            target_status: DplTarget::Deployed,
            ..Default::default()
        }
    }

    #[test]
    fn converts_deployed_event() {
        let t = fixed_time();
        let dpl = Deployment {
            deployed_at: Some(t),
            ..deployment()
        };
        let event = stored(7, EventArgs::deployed(&dpl).unwrap());

        let data = openapi::DeploymentDeployedEvent {
            deployment_id: "dpl-1".into(),
            release_id: "rls-1".into(),
            status: openapi::DeploymentStatus::DEPLOYMENT_STATUS_DEPLOYED,
            activity_status: openapi::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DEPLOYED,
            error_status: openapi::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_NONE,
            target_status: openapi::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_DEPLOYED,
            deployed_at: Some(t.to_rfc3339()),
        };
        let expected = openapi::Event {
            object: openapi::event::Object::Event,
            id: 7,
            r#type: "deployment.deployed".into(),
            occurred_at: t.to_rfc3339(),
            data: serde_json::to_value(data).unwrap(),
        };

        let sdk = openapi::Event::try_from(&event).unwrap();
        assert_eq!(sdk, expected);
    }

    #[test]
    fn omits_absent_deployed_at() {
        let event = stored(7, EventArgs::deployed(&deployment()).unwrap());

        let sdk = openapi::Event::try_from(&event).unwrap();
        assert!(sdk.data.get("deployed_at").is_none());
    }

    #[test]
    fn omits_absent_archived_at() {
        let event = stored(8, EventArgs::removed(&deployment()).unwrap());

        let sdk = openapi::Event::try_from(&event).unwrap();
        assert!(sdk.data.get("archived_at").is_none());
    }

    #[test]
    fn converts_removed_event() {
        let t = fixed_time();
        let dpl = Deployment {
            activity_status: DplActivity::Archived,
            target_status: DplTarget::Archived,
            archived_at: Some(t),
            ..deployment()
        };
        let event = stored(8, EventArgs::removed(&dpl).unwrap());

        let data = openapi::DeploymentRemovedEvent {
            deployment_id: "dpl-1".into(),
            release_id: "rls-1".into(),
            status: openapi::DeploymentStatus::DEPLOYMENT_STATUS_ARCHIVED,
            activity_status: openapi::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_ARCHIVED,
            error_status: openapi::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_NONE,
            target_status: openapi::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_ARCHIVED,
            archived_at: Some(t.to_rfc3339()),
        };
        let expected = openapi::Event {
            object: openapi::event::Object::Event,
            id: 8,
            r#type: "deployment.removed".into(),
            occurred_at: t.to_rfc3339(),
            data: serde_json::to_value(data).unwrap(),
        };

        let sdk = openapi::Event::try_from(&event).unwrap();
        assert_eq!(sdk, expected);
    }

    #[test]
    fn rejects_unknown_event_type() {
        let event = Event {
            id: 9,
            event_type: "deployment.unknown".into(),
            occurred_at: fixed_time(),
            data: serde_json::json!({}),
        };

        let result = openapi::Event::try_from(&event);
        assert!(matches!(result, Err(EventsErr::UnknownEventTypeErr(_))));
    }
}

pub mod create_calibration_response {
    use super::*;

    #[test]
    fn converts_created() {
        let t = fixed_time();
        let report_id = Uuid::new_v4();
        let created = Created {
            report_id,
            reported_at: t,
        };

        let expected = openapi::CreateCalibrationResponse {
            report_id,
            reported_at: t.to_rfc3339(),
        };

        let sdk: openapi::CreateCalibrationResponse = (&created).into();
        assert_eq!(sdk, expected);
    }
}

pub mod latest_calibrations_response {
    use super::*;
    use miru_daemon::calibrations::ReportMetadata;
    use miru_daemon::models::InstanceFormat;
    use miru_daemon::server::v03::response::latest_calibrations;

    fn metadata(report_id: Uuid) -> ReportMetadata {
        ReportMetadata {
            report_id,
            component: "arm".into(),
            format: InstanceFormat::Yaml,
            role: Some("left".into()),
            aspect: None,
            serial_number: Some("SN-1".into()),
            reported_by: Some("Ada Lovelace".into()),
            deployment_id: Some("dpl_1".into()),
            reported_at: fixed_time(),
            invalidated_at: None,
        }
    }

    #[test]
    fn converts_report_metadata() {
        let report_id = Uuid::new_v4();
        let metadata = metadata(report_id);

        let expected = openapi::Calibration {
            object: openapi::calibration::Object::Calibration,
            report_id,
            component: "arm-left".into(),
            serial_number: Some("SN-1".into()),
            reported_by: Some("Ada Lovelace".into()),
            reported_at: fixed_time().to_rfc3339(),
            invalidated_at: None,
        };

        let sdk: openapi::Calibration = (&metadata).into();
        assert_eq!(sdk, expected);
    }

    #[test]
    fn converts_invalidated_at() {
        let invalidated_at = fixed_time() + Duration::hours(1);
        let metadata = ReportMetadata {
            invalidated_at: Some(invalidated_at),
            ..metadata(Uuid::new_v4())
        };

        let sdk: openapi::Calibration = (&metadata).into();
        assert_eq!(sdk.invalidated_at, Some(invalidated_at.to_rfc3339()));
    }

    #[test]
    fn converts_latest_calibrations_in_order() {
        let (first, second) = (Uuid::new_v4(), Uuid::new_v4());

        let response = latest_calibrations(&[metadata(first), metadata(second)]);

        let calibrations = response.calibrations;
        let ids: Vec<Uuid> = calibrations.iter().map(|c| c.report_id).collect();
        assert_eq!(ids, [first, second]);
    }
}
