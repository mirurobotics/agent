// standard crates
use std::sync::atomic::{AtomicUsize, Ordering};

// internal crates
use miru_agent::deploy::{apply, fsm};
use miru_agent::filesys::{self, Overwrite};
use miru_agent::http::errors::*;
use miru_agent::models::{self, DplActivity, DplErrStatus, DplTarget};
use miru_agent::storage::{self, CfgInstContent, CfgInsts, Deployments, GitCommits, Releases};
use miru_agent::sync::deployments::{sync, SyncArgs};
use miru_agent::sync::SyncErr;

// test crates
use crate::http::mock::{Call, CapturedRequest, MockClient};
use crate::sync::helpers::*;
use openapi_client::models::{
    Deployment as BackendDeployment, DeploymentActivityStatus as BackendActivityStatus,
    DeploymentErrorStatus as BackendErrorStatus, DeploymentTargetStatus as BackendTargetStatus,
    UpdateDeploymentRequest,
};

// external crates
use chrono::{DateTime, TimeDelta, Utc};

// ========================= FIXTURE ========================= //

struct Fixture {
    deployment_stor: Deployments,
    cfg_inst_stor: CfgInsts,
    cfg_inst_content_stor: CfgInstContent,
    release_stor: Releases,
    git_commit_stor: GitCommits,
    http_client: MockClient,
    staging_dir: filesys::Dir,
    target_dir: filesys::Dir,
    retry_policy: fsm::RetryPolicy,
    _dir: filesys::Dir,
}

impl Fixture {
    async fn new(name: &str) -> Self {
        let dir = filesys::Dir::create_temp_dir(name).await.unwrap();
        let (deployment_stor, _) = Deployments::spawn(16, dir.file("deployments.json"), 1000)
            .await
            .unwrap();
        let (cfg_inst_stor, _) = CfgInsts::spawn(16, dir.file("cfg_insts.json"), 1000)
            .await
            .unwrap();
        let (cfg_inst_content_stor, _) =
            CfgInstContent::spawn(16, dir.subdir("cfg_inst_content"), 1000)
                .await
                .unwrap();
        let (release_stor, _) = Releases::spawn(16, dir.file("releases.json"), 1000)
            .await
            .unwrap();
        let (git_commit_stor, _) = GitCommits::spawn(16, dir.file("git_commits.json"), 1000)
            .await
            .unwrap();
        Self {
            deployment_stor,
            cfg_inst_stor,
            cfg_inst_content_stor,
            release_stor,
            git_commit_stor,
            http_client: MockClient::default(),
            staging_dir: dir.subdir("staging"),
            target_dir: dir.subdir("deployments"),
            retry_policy: fsm::RetryPolicy::default(),
            _dir: dir,
        }
    }

    async fn sync(&self) -> Result<Option<TimeDelta>, SyncErr> {
        let opts = apply::DeployOpts {
            staging_dir: self.staging_dir.clone(),
            target_dir: self.target_dir.clone(),
            retry_policy: self.retry_policy,
        };
        sync(&SyncArgs {
            storage: &miru_agent::sync::deployments::Storage {
                deployments: &self.deployment_stor,
                cfg_insts: storage::CfgInstRef {
                    meta: &self.cfg_inst_stor,
                    content: &self.cfg_inst_content_stor,
                },
                releases: &self.release_stor,
                git_commits: &self.git_commit_stor,
            },
            http_client: &self.http_client,
            opts: &opts,
            token: "test_token",
        })
        .await
    }
}

fn push_bodies(requests: &[CapturedRequest]) -> Vec<UpdateDeploymentRequest> {
    requests
        .iter()
        .filter(|r| r.call == Call::UpdateDeployment)
        .map(|r| {
            serde_json::from_str(r.body.as_deref().expect("push should have a body"))
                .expect("push body should deserialize as UpdateDeploymentRequest")
        })
        .collect()
}

// ========================= TESTS ========================= //

#[tokio::test]
async fn empty_sync_returns_none() {
    let f = Fixture::new("sync_empty_none").await;
    assert_eq!(f.sync().await.unwrap(), None);
}

mod pull_success {
    use super::*;

    #[tokio::test]
    async fn stores_deployment_and_config_instances() {
        let f = Fixture::new("stores_deployment_and_config_instances").await;
        let backend_dep = make_deployment("dpl_1", &["cfg_inst_1", "cfg_inst_2"]);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));

        f.sync().await.unwrap();

        assert_deployment_stored(&f.deployment_stor, "dpl_1").await;
        assert_cfg_inst_stored(&f.cfg_inst_stor, "cfg_inst_1").await;
        assert_cfg_inst_stored(&f.cfg_inst_stor, "cfg_inst_2").await;
    }

    #[tokio::test]
    async fn stores_multiple_deployments() {
        let f = Fixture::new("stores_multiple_deployments").await;
        let dpl_1 = make_deployment("dpl_1", &["cfg_inst_a", "cfg_inst_c"]);
        let dpl_2 = make_archived_dpl("dpl_2", &["cfg_inst_b", "cfg_inst_d"]);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![dpl_1.clone(), dpl_2.clone()]));
        f.sync().await.unwrap();

        // deployments stored
        assert_deployment_stored(&f.deployment_stor, "dpl_1").await;
        assert_deployment_stored(&f.deployment_stor, "dpl_2").await;

        // config instances stored
        assert_cfg_inst_stored(&f.cfg_inst_stor, "cfg_inst_a").await;
        assert_cfg_inst_stored(&f.cfg_inst_stor, "cfg_inst_b").await;
        assert_cfg_inst_stored(&f.cfg_inst_stor, "cfg_inst_c").await;
        assert_cfg_inst_stored(&f.cfg_inst_stor, "cfg_inst_d").await;
    }

    #[tokio::test]
    async fn stores_release_and_git_commit_from_expanded_deployment() {
        let f = Fixture::new("sync_release_gc").await;
        let dpl = make_deployment_with_release("dpl_1", &["cfg_inst_1"], "rel_1", Some("gc_1"));
        f.http_client
            .set_list_all_deployments(move || Ok(vec![dpl.clone()]));

        f.sync().await.unwrap();

        assert_deployment_stored(&f.deployment_stor, "dpl_1").await;
        assert_release_stored(&f.release_stor, "rel_1").await;
        assert_git_commit_stored(&f.git_commit_stor, "gc_1").await;
    }

    #[tokio::test]
    async fn stores_release_without_git_commit() {
        let f = Fixture::new("sync_release_no_gc").await;
        let dpl = make_deployment_with_release("dpl_1", &["cfg_inst_1"], "rel_2", None);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![dpl.clone()]));

        f.sync().await.unwrap();

        assert_deployment_stored(&f.deployment_stor, "dpl_1").await;
        assert_release_stored(&f.release_stor, "rel_2").await;

        // git commit should not be stored when release has no git_commit
        let gc = f.git_commit_stor.values().await.unwrap();
        assert!(gc.is_empty(), "no git commit should be stored");
    }

    #[tokio::test]
    async fn fetches_content_individually() {
        let f = Fixture::new("sync_content_individual").await;
        let backend_dep = make_deployment("dpl_1", &["cfg_inst_1", "cfg_inst_2"]);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));
        f.http_client
            .set_get_config_instance_content(|id| match id {
                "cfg_inst_1" => Ok(r#"{"alpha": true}"#.to_string()),
                "cfg_inst_2" => Ok("beta: yes".to_string()),
                other => panic!("unexpected config instance id: {other}"),
            });

        f.sync().await.unwrap();

        let content_1 = read_content(&f.cfg_inst_content_stor, "cfg_inst_1").await;
        assert_eq!(content_1, r#"{"alpha": true}"#);

        let content_2 = read_content(&f.cfg_inst_content_stor, "cfg_inst_2").await;
        assert_eq!(content_2, "beta: yes");

        assert_eq!(f.http_client.call_count(Call::GetConfigInstanceContent), 2);
    }

    #[tokio::test]
    async fn content_already_cached_skips_fetch() {
        let f = Fixture::new("sync_content_cached").await;
        let backend_dep = make_deployment("dpl_1", &["cfg_inst_1"]);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));

        f.cfg_inst_content_stor
            .write(
                "cfg_inst_1".to_string(),
                "old content".to_string(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();
        f.sync().await.unwrap();

        // content fetch should not have been called
        assert_eq!(
            f.http_client.call_count(Call::GetConfigInstanceContent),
            0,
            "should skip HTTP fetch when content is already cached"
        );

        // cached content unchanged
        let content = read_content(&f.cfg_inst_content_stor, "cfg_inst_1").await;
        assert_eq!(content, "old content");
    }

    #[tokio::test]
    async fn merge_preserves_agent_fields() {
        let f = Fixture::new("sync_merge").await;

        // seed a deployment with agent-side fields set
        let future_cooldown = Utc::now() + chrono::TimeDelta::seconds(3600);
        let seeded = models::Deployment {
            id: "dpl_1".to_string(),
            activity_status: DplActivity::Deployed,
            error_status: DplErrStatus::None,
            target_status: DplTarget::Deployed,
            created_at: DateTime::<Utc>::UNIX_EPOCH,
            config_instance_ids: vec!["cfg_inst_1".to_string()],
            attempts: 5,
            cooldown_ends_at: future_cooldown,
            ..Default::default()
        };
        f.deployment_stor
            .write(
                "dpl_1".to_string(),
                seeded.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        let backend_dep = make_deployment("dpl_1", &["cfg_inst_1", "cfg_inst_2"]);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));

        f.sync().await.unwrap();

        let cached = read_deployment(&f.deployment_stor, "dpl_1").await;
        assert_eq!(seeded, cached);
    }

    #[tokio::test]
    async fn pull_preserves_existing_dirty_flag_for_push() {
        let f = Fixture::new("pull_preserves_dirty_flag").await;

        // seed a non-actionable deployment and explicitly mark it dirty. This ensures
        // any update push is due to dirty-flag preservation across pull, not apply().
        let seeded = models::Deployment {
            id: "dpl_1".to_string(),
            activity_status: DplActivity::Staged,
            error_status: DplErrStatus::None,
            target_status: DplTarget::Staged,
            config_instance_ids: vec!["cfg_inst_1".to_string()],
            ..Default::default()
        };
        f.deployment_stor
            .write(
                "dpl_1".to_string(),
                seeded,
                |_, _| true, // force dirty for this test
                Overwrite::Allow,
            )
            .await
            .unwrap();

        let backend_dep = BackendDeployment {
            activity_status: BackendActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_STAGED,
            target_status: BackendTargetStatus::DEPLOYMENT_TARGET_STATUS_STAGED,
            ..make_deployment("dpl_1", &["cfg_inst_1"])
        };
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));

        f.sync().await.unwrap();

        assert_eq!(
            f.http_client.call_count(Call::UpdateDeployment),
            1,
            "dirty deployment should still be pushed after pull reconciliation"
        );
    }

    #[tokio::test]
    async fn shared_config_instance_across_deployments() {
        let f = Fixture::new("sync_shared_cfg_inst").await;

        // both deployments reference the same config instance
        let dpl_1 = make_deployment("dpl_1", &["shared_cfg_inst"]);
        let dpl_2 = make_archived_dpl("dpl_2", &["shared_cfg_inst"]);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![dpl_1.clone(), dpl_2.clone()]));
        f.http_client
            .set_get_config_instance_content(|_id| Ok("shared content".to_string()));
        f.sync().await.unwrap();

        // content fetched only once — second fetch skipped by cache
        assert_eq!(
            f.http_client.call_count(Call::GetConfigInstanceContent),
            1,
            "shared config instance content should be fetched only once"
        );

        let content = read_content(&f.cfg_inst_content_stor, "shared_cfg_inst").await;
        assert_eq!(content, "shared content");
    }

    #[tokio::test]
    async fn pull_deployments_retries_on_network_error() {
        let f = Fixture::new("sync_retry_pull").await;
        let call_count = AtomicUsize::new(0);
        let backend_dep = make_deployment("dpl_1", &["cfg_inst_1"]);
        f.http_client.set_list_all_deployments(move || {
            let n = call_count.fetch_add(1, Ordering::SeqCst);
            if n < 2 {
                Err(HTTPErr::MockErr(MockErr {
                    is_network_conn_err: true,
                }))
            } else {
                Ok(vec![backend_dep.clone()])
            }
        });

        f.sync().await.unwrap();

        // 3 total list deployments calls (2 failures + 1 success)
        assert_eq!(f.http_client.call_count(Call::ListDeployments), 3);

        // deployment was stored
        assert_deployment_stored(&f.deployment_stor, "dpl_1").await;
    }

    #[tokio::test]
    async fn pull_deployments_no_retry_on_app_error() {
        let f = Fixture::new("sync_retry_no_app").await;
        f.http_client.set_list_all_deployments(|| {
            Err(HTTPErr::MockErr(MockErr {
                is_network_conn_err: false,
            }))
        });

        f.sync().await.unwrap_err();
        assert_eq!(f.http_client.call_count(Call::ListDeployments), 1);

        // deployment was not stored
        assert_deployment_not_stored(&f.deployment_stor, "dpl_1").await;
    }

    #[tokio::test]
    async fn pull_deployments_exhausts_retries() {
        let f = Fixture::new("sync_retry_exhaust").await;
        f.http_client.set_list_all_deployments(|| {
            Err(HTTPErr::MockErr(MockErr {
                is_network_conn_err: true,
            }))
        });

        let result = f.sync().await;
        assert!(result.is_err());

        // 3 total attempts (1 initial + 2 retries)
        assert_eq!(
            f.http_client.call_count(Call::ListDeployments),
            3,
            "should make 3 total attempts before giving up"
        );
    }
}

mod pull_failure {
    use super::*;

    #[tokio::test]
    async fn cfg_inst_not_expanded_error() {
        let f = Fixture::new("cfg_insts_not_expanded_error").await;
        let unexpanded = BackendDeployment {
            config_instances: None,
            ..make_deployment("dpl_1", &["cfg_inst_1", "cfg_inst_2"])
        };
        f.http_client
            .set_list_all_deployments(move || Ok(vec![unexpanded.clone()]));

        let err = f.sync().await.unwrap_err();
        let is_cfg_insts_not_expanded = matches!(err, SyncErr::CfgInstsNotExpanded(_))
            || matches!(
                &err,
                SyncErr::SyncErrors(se) if se.errors.iter().any(|e| matches!(e, SyncErr::CfgInstsNotExpanded(_)))
            );
        assert!(
            is_cfg_insts_not_expanded,
            "expected CfgInstsNotExpanded (or SyncErrors containing it), got: {err:?}"
        );
    }

    #[tokio::test]
    async fn content_partial_failure_fetches_remaining() {
        let f = Fixture::new("sync_content_partial").await;
        let backend_dep = make_deployment("dpl_1", &["cfg_inst_1", "cfg_inst_2"]);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));
        f.http_client
            .set_get_config_instance_content(|id| match id {
                "cfg_inst_1" => Err(HTTPErr::MockErr(MockErr {
                    is_network_conn_err: false,
                })),
                "cfg_inst_2" => Ok("success content".to_string()),
                other => panic!("unexpected config instance id: {other}"),
            });

        let err = f.sync().await.unwrap_err();

        // content not stored for failed fetch
        assert_content_not_stored(&f.cfg_inst_content_stor, "cfg_inst_1").await;
        let content_2 = read_content(&f.cfg_inst_content_stor, "cfg_inst_2").await;
        assert_eq!(content_2, "success content");

        // both content fetches were attempted
        assert_eq!(
            f.http_client.call_count(Call::GetConfigInstanceContent),
            2,
            "both content fetches should be attempted"
        );

        // sync returns error with exactly 1 inner error
        match err {
            SyncErr::SyncErrors(se) => {
                let content_err = se
                    .errors
                    .iter()
                    .find(|e| matches!(e, SyncErr::SyncErrors(_)));
                assert!(content_err.is_some(), "should contain content pull error");
                if let Some(SyncErr::SyncErrors(inner)) = content_err {
                    assert_eq!(inner.errors.len(), 1, "exactly 1 content fetch error");
                }
            }
            other => panic!("expected SyncErrors, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn content_all_fetches_fail() {
        let f = Fixture::new("sync_content_all_fail").await;
        let backend_dep = make_deployment("dpl_1", &["cfg_inst_1", "cfg_inst_2"]);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));
        f.http_client.set_get_config_instance_content(|_id| {
            Err(HTTPErr::MockErr(MockErr {
                is_network_conn_err: false,
            }))
        });

        let err = f.sync().await.unwrap_err();

        // content not stored for failed fetches
        assert_content_not_stored(&f.cfg_inst_content_stor, "cfg_inst_1").await;
        assert_content_not_stored(&f.cfg_inst_content_stor, "cfg_inst_2").await;

        // both content fetches were attempted
        assert_eq!(
            f.http_client.call_count(Call::GetConfigInstanceContent),
            2,
            "both content fetches should be attempted"
        );

        // error contains both failures
        match err {
            SyncErr::SyncErrors(se) => {
                let content_err = se
                    .errors
                    .iter()
                    .find(|e| matches!(e, SyncErr::SyncErrors(_)));
                assert!(content_err.is_some());
                if let Some(SyncErr::SyncErrors(inner)) = content_err {
                    assert_eq!(inner.errors.len(), 2, "both content fetch errors collected");
                }
            }
            other => panic!("expected SyncErrors, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn failure_still_applies_and_pushes() {
        let f = Fixture::new("sync_pull_fail_push").await;
        let backend_dep = make_deployment("dpl_1", &["cfg_inst_1", "cfg_inst_2"]);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));
        f.http_client.set_get_config_instance_content(|_id| {
            Err(HTTPErr::MockErr(MockErr {
                is_network_conn_err: false,
            }))
        });

        let err = f.sync().await.unwrap_err();
        assert!(
            matches!(err, SyncErr::SyncErrors(_)),
            "error should be SyncErrors"
        );
        assert!(
            f.http_client.call_count(Call::UpdateDeployment) > 0,
            "push should still execute despite pull content failure"
        );
    }

    #[tokio::test]
    async fn multiple_deployments_partial_content_failure() {
        let f = Fixture::new("sync_multi_dpl_partial").await;
        let dpl_1 = make_archived_dpl("dpl_1", &["cfg_inst_a"]);
        let dpl_2 = make_deployment("dpl_2", &["cfg_inst_b"]);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![dpl_1.clone(), dpl_2.clone()]));
        f.http_client
            .set_get_config_instance_content(|id| match id {
                "cfg_inst_a" => Err(HTTPErr::MockErr(MockErr {
                    is_network_conn_err: false,
                })),
                "cfg_inst_b" => Ok("b content".to_string()),
                other => panic!("unexpected config instance id: {other}"),
            });

        f.sync().await.unwrap_err();

        // cfg_inst_b stored despite cfg_inst_a failure
        let content_b = read_content(&f.cfg_inst_content_stor, "cfg_inst_b").await;
        assert_eq!(content_b, "b content");

        // content not stored for failed fetch
        assert_content_not_stored(&f.cfg_inst_content_stor, "cfg_inst_a").await;
    }

    #[tokio::test]
    async fn metadata_stored_but_content_missing_recovers_on_next_sync() {
        let f = Fixture::new("sync_content_recovery").await;
        let backend_dep = make_deployment("dpl_1", &["cfg_inst_1"]);

        let call_count = std::sync::atomic::AtomicUsize::new(0);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));
        f.http_client.set_get_config_instance_content(move |_id| {
            let n = call_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                // won't be retried since non-network error
                Err(HTTPErr::MockErr(MockErr {
                    is_network_conn_err: false,
                }))
            } else {
                Ok("recovered content".to_string())
            }
        });

        // sync 1: content fetch fails
        f.sync().await.unwrap_err();
        assert_content_not_stored(&f.cfg_inst_content_stor, "cfg_inst_1").await;

        // sync 2: content fetch succeeds
        f.sync().await.unwrap();
        let content = read_content(&f.cfg_inst_content_stor, "cfg_inst_1").await;
        assert_eq!(content, "recovered content");
    }

    #[tokio::test]
    async fn deployment_list_failure_still_pulls_content_for_cached_deployments() {
        let f = Fixture::new("sync_list_fail_cached").await;

        // pre-seed a deployment in storage
        let seeded = models::Deployment {
            id: "dpl_1".to_string(),
            activity_status: DplActivity::Queued,
            error_status: DplErrStatus::None,
            target_status: DplTarget::Deployed,
            config_instance_ids: vec!["cfg_inst_1".to_string()],
            ..Default::default()
        };
        f.deployment_stor
            .write("dpl_1".to_string(), seeded, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();

        // pre-seed config instance metadata
        let cfg_inst_meta = models::ConfigInstance {
            id: "cfg_inst_1".to_string(),
            ..Default::default()
        };
        f.cfg_inst_stor
            .write(
                "cfg_inst_1".to_string(),
                cfg_inst_meta,
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        // list deployments fails (non-network, so no retry)
        f.http_client.set_list_all_deployments(|| {
            Err(HTTPErr::MockErr(MockErr {
                is_network_conn_err: false,
            }))
        });
        f.http_client
            .set_get_config_instance_content(|_id| Ok("fetched content".to_string()));

        f.sync().await.unwrap_err();

        // content was fetched for the cached deployment's config instance
        let content = read_content(&f.cfg_inst_content_stor, "cfg_inst_1").await;
        assert_eq!(content, "fetched content");
        assert_eq!(f.http_client.call_count(Call::GetConfigInstanceContent), 1);
    }
}

pub mod apply_success {
    use super::*;

    #[tokio::test]
    async fn deploy_queued_deployment() {
        let f = Fixture::new("apply_deploy_queued").await;
        let backend_dep = make_deployment("dpl_1", &["cfg_inst_1"]);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));

        f.sync().await.unwrap();

        let cached = read_deployment(&f.deployment_stor, "dpl_1").await;
        assert_eq!(cached.activity_status, DplActivity::Deployed);
        assert_eq!(cached.error_status, DplErrStatus::None);
    }

    #[tokio::test]
    async fn archive_queued_deployment() {
        let f = Fixture::new("apply_archive_queued").await;
        let backend_dep = make_archived_dpl("dpl_1", &["cfg_inst_1"]);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));

        f.sync().await.unwrap();

        let cached = read_deployment(&f.deployment_stor, "dpl_1").await;
        assert_eq!(cached.activity_status, DplActivity::Archived);
        assert_eq!(cached.error_status, DplErrStatus::None);
    }

    #[tokio::test]
    async fn sync_returns_wait_for_deployment_in_cooldown() {
        let f = Fixture::new("sync_wait_cooldown").await;

        let future_cooldown = Utc::now() + TimeDelta::seconds(30);
        let seeded = models::Deployment {
            id: "dpl_1".to_string(),
            activity_status: DplActivity::Queued,
            error_status: DplErrStatus::Retrying,
            target_status: DplTarget::Deployed,
            config_instance_ids: vec!["cfg_inst_1".to_string()],
            cooldown_ends_at: future_cooldown,
            ..Default::default()
        };
        f.deployment_stor
            .write("dpl_1".to_string(), seeded, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();

        // pre-cache content so content pull doesn't fail
        f.cfg_inst_content_stor
            .write(
                "cfg_inst_1".to_string(),
                "{}".to_string(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        // backend returns same deployment (resolve_dpl preserves cooldown)
        let backend_dep = make_deployment("dpl_1", &["cfg_inst_1", "cfg_inst_2"]);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));

        let result = f.sync().await;
        let wait = result.unwrap();
        assert!(
            wait.is_some(),
            "should return Some(wait) for deployment in cooldown"
        );
        assert!(wait.unwrap() > TimeDelta::zero(), "wait should be positive");
    }

    #[tokio::test]
    async fn multiple_deployments_min_wait_returned() {
        let f = Fixture::new("sync_min_wait").await;

        // Deployment A: target=Deployed, error=Retrying, cooldown=30s
        let cooldown_a = Utc::now() + TimeDelta::seconds(30);
        let seeded_a = models::Deployment {
            id: "dpl_a".to_string(),
            activity_status: DplActivity::Queued,
            error_status: DplErrStatus::Retrying,
            target_status: DplTarget::Deployed,
            config_instance_ids: vec!["cfg_inst_a".to_string()],
            cooldown_ends_at: cooldown_a,
            ..Default::default()
        };
        f.deployment_stor
            .write(
                "dpl_a".to_string(),
                seeded_a,
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        // Deployment B: target=Archived, error=Retrying, cooldown=120s
        let cooldown_b = Utc::now() + TimeDelta::seconds(120);
        let seeded_b = models::Deployment {
            id: "dpl_b".to_string(),
            activity_status: DplActivity::Deployed,
            error_status: DplErrStatus::Retrying,
            target_status: DplTarget::Archived,
            config_instance_ids: vec!["cfg_inst_b".to_string()],
            cooldown_ends_at: cooldown_b,
            ..Default::default()
        };
        f.deployment_stor
            .write(
                "dpl_b".to_string(),
                seeded_b,
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        // pre-cache content so content pull doesn't fail
        f.cfg_inst_content_stor
            .write(
                "cfg_inst_a".to_string(),
                "{}".to_string(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();
        f.cfg_inst_content_stor
            .write(
                "cfg_inst_b".to_string(),
                "{}".to_string(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        // backend returns both deployments (different targets to avoid ConflictingDeployments)
        let backend_a = make_deployment("dpl_a", &["cfg_inst_a"]);
        let backend_b = make_archived_dpl("dpl_b", &["cfg_inst_b"]);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_a.clone(), backend_b.clone()]));

        let result = f.sync().await;
        let wait = result.unwrap();
        assert!(
            wait.is_some(),
            "should return Some(wait) when deployments are in cooldown"
        );
        let wait = wait.unwrap();
        // The smaller cooldown (~30s) should be returned, not the larger (~120s)
        assert!(
            wait <= TimeDelta::seconds(31),
            "wait should be ~30s (the minimum), got: {wait}"
        );
        assert!(wait > TimeDelta::zero(), "wait should be positive");
    }
}

pub mod apply_failure {
    use super::*;
    use miru_agent::deploy::errors::DeployErr;

    #[tokio::test]
    async fn conflicting_deployments() {
        let f = Fixture::new("apply_conflicting").await;
        let dpl_1 = make_deployment("dpl_1", &["cfg_inst_1"]);
        let dpl_2 = make_deployment("dpl_2", &["cfg_inst_2"]);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![dpl_1.clone(), dpl_2.clone()]));

        let err = f.sync().await.unwrap_err();
        let has_conflict = matches!(
            &err,
            SyncErr::SyncErrors(se) if se.errors.iter().any(|e|
                matches!(e, SyncErr::DeployErr(de) if matches!(de.as_ref(), DeployErr::ConflictingDeployments(_)))
            )
        );
        assert!(
            has_conflict,
            "expected ConflictingDeployments error, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn content_missing_triggers_retrying() {
        let f = Fixture::new("apply_content_missing").await;
        let backend_dep = make_deployment("dpl_1", &["cfg_inst_1"]);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));
        f.http_client.set_get_config_instance_content(|_id| {
            Err(HTTPErr::MockErr(MockErr {
                is_network_conn_err: false,
            }))
        });

        f.sync().await.unwrap_err();

        let cached = read_deployment(&f.deployment_stor, "dpl_1").await;
        assert_eq!(
            cached.error_status,
            DplErrStatus::Retrying,
            "FSM should transition to Retrying on deploy error"
        );
        assert!(cached.attempts > 0, "attempts should be bumped");
        assert!(
            cached.cooldown_ends_at > DateTime::<Utc>::UNIX_EPOCH,
            "cooldown should be set"
        );
    }
}

mod push_success {
    use super::*;

    #[tokio::test]
    async fn sends_dirty_deployments() {
        let f = Fixture::new("sync_push").await;
        let backend_dep = make_deployment("dpl_1", &["cfg_inst_1"]);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));

        f.sync().await.unwrap();
        assert!(
            f.http_client.call_count(Call::UpdateDeployment) == 1,
            "push should send UpdateDeployment for dirty deployments"
        );

        // sending a clean deployment should not send an UpdateDeployment
        f.sync().await.unwrap();
        assert!(
            f.http_client.call_count(Call::UpdateDeployment) == 1,
            "push should not send UpdateDeployment for clean deployments"
        );
    }

    #[tokio::test]
    async fn dirty_flag_preserved_on_failure() {
        let f = Fixture::new("sync_push_dirty_preserved").await;
        let backend_dep = make_deployment("dpl_1", &["cfg_inst_1"]);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));
        f.http_client.set_update_deployment(|| {
            Err(HTTPErr::MockErr(MockErr {
                is_network_conn_err: false,
            }))
        });

        // first sync: push fails, dirty flag should remain
        f.sync().await.unwrap_err();
        let calls_after_first_sync = f.http_client.call_count(Call::UpdateDeployment);
        assert_eq!(
            calls_after_first_sync, 1,
            "first sync should attempt one deployment push"
        );

        let dirty = f.deployment_stor.get_dirty_entries().await.unwrap();
        assert_eq!(
            dirty.len(),
            1,
            "deployment should still be dirty after push failure"
        );
        assert_eq!(dirty[0].value.id, "dpl_1");

        // second sync with fixed push: should push again
        f.http_client
            .set_update_deployment(|| Ok(BackendDeployment::default()));
        let result2 = f.sync().await;
        assert!(result2.is_ok(), "second sync should succeed");
        assert_eq!(
            f.http_client.call_count(Call::UpdateDeployment),
            calls_after_first_sync + 1,
            "second sync should retry deployment push exactly once"
        );

        // now no longer dirty
        let dirty2 = f.deployment_stor.get_dirty_entries().await.unwrap();
        assert!(
            dirty2.is_empty(),
            "deployment should be clean after successful push"
        );
    }

    #[tokio::test]
    async fn successful_push_clears_dirty_flag() {
        let f = Fixture::new("sync_push_clears_dirty").await;
        let backend_dep = make_deployment("dpl_1", &["cfg_inst_1"]);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));

        f.sync().await.unwrap();

        let dirty = f.deployment_stor.get_dirty_entries().await.unwrap();
        assert!(
            dirty.is_empty(),
            "deployment should be clean after successful push"
        );
    }
}

mod push_failure {
    use super::*;

    #[tokio::test]
    async fn error_accumulation() {
        let f = Fixture::new("sync_push_err").await;
        let backend_dep = make_deployment("dpl_1", &["cfg_inst_1"]);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));
        f.http_client.set_update_deployment(|| {
            Err(HTTPErr::MockErr(MockErr {
                is_network_conn_err: false,
            }))
        });

        let err = f.sync().await.unwrap_err();
        assert!(
            matches!(err, SyncErr::SyncErrors(_)),
            "error should be SyncErrors"
        );
    }

    #[tokio::test]
    async fn retries_on_network_error() {
        let f = Fixture::new("push_retry_network").await;
        let backend_dep = make_deployment("dpl_1", &["cfg_inst_1"]);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));

        let push_count = AtomicUsize::new(0);
        f.http_client.set_update_deployment(move || {
            let n = push_count.fetch_add(1, Ordering::SeqCst);
            if n < 2 {
                Err(HTTPErr::MockErr(MockErr {
                    is_network_conn_err: true,
                }))
            } else {
                Ok(BackendDeployment::default())
            }
        });

        f.sync().await.unwrap();

        // 3 total update deployment calls (2 failures + 1 success)
        assert_eq!(f.http_client.call_count(Call::UpdateDeployment), 3);
    }

    #[tokio::test]
    async fn no_retry_on_app_error() {
        let f = Fixture::new("push_no_retry_app").await;
        let backend_dep = make_deployment("dpl_1", &["cfg_inst_1"]);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));
        f.http_client.set_update_deployment(|| {
            Err(HTTPErr::MockErr(MockErr {
                is_network_conn_err: false,
            }))
        });

        f.sync().await.unwrap_err();

        // only 1 update deployment call — app errors are not retried
        assert_eq!(
            f.http_client.call_count(Call::UpdateDeployment),
            1,
            "app errors should not be retried"
        );
    }

    #[tokio::test]
    async fn exhausts_retries() {
        let f = Fixture::new("push_retry_exhaust").await;
        let backend_dep = make_deployment("dpl_1", &["cfg_inst_1"]);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));
        f.http_client.set_update_deployment(|| {
            Err(HTTPErr::MockErr(MockErr {
                is_network_conn_err: true,
            }))
        });

        f.sync().await.unwrap_err();

        // 3 total update deployment calls (1 initial + 2 retries)
        assert_eq!(
            f.http_client.call_count(Call::UpdateDeployment),
            3,
            "should make 3 total attempts before giving up"
        );
    }

    #[tokio::test]
    async fn multiple_deployments_push_partial_failure() {
        let f = Fixture::new("push_partial_multi").await;

        // backend returns 2 deployments: one to deploy, one to archive
        let dpl_deploy = make_deployment("dpl_1", &["cfg_inst_1"]);
        let dpl_archive = make_archived_dpl("dpl_2", &["cfg_inst_2"]);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![dpl_deploy.clone(), dpl_archive.clone()]));
        f.http_client
            .set_get_config_instance_content(|_id| Ok("{}".to_string()));

        // push: fail first call, succeed second (non-network so no retry)
        let push_count = AtomicUsize::new(0);
        f.http_client.set_update_deployment(move || {
            let n = push_count.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Err(HTTPErr::MockErr(MockErr {
                    is_network_conn_err: false,
                }))
            } else {
                Ok(BackendDeployment::default())
            }
        });

        // first sync: one push fails, one succeeds
        f.sync().await.unwrap_err();
        assert_eq!(
            f.http_client.call_count(Call::UpdateDeployment),
            2,
            "both deployments should attempt push"
        );

        // one deployment should still be dirty (the one whose push failed)
        let dirty = f.deployment_stor.get_dirty_entries().await.unwrap();
        assert_eq!(
            dirty.len(),
            1,
            "exactly one deployment should remain dirty after partial push failure"
        );

        // second sync: only the previously-failed dirty entry re-pushed
        let result2 = f.sync().await;
        assert!(result2.is_ok(), "second sync should succeed");
        assert_eq!(
            f.http_client.call_count(Call::UpdateDeployment),
            3,
            "second sync should push only the previously-failed deployment (2 + 1 = 3 total)"
        );

        let dirty2 = f.deployment_stor.get_dirty_entries().await.unwrap();
        assert!(
            dirty2.is_empty(),
            "all deployments should be clean after successful retry"
        );
    }
}

mod cross_phase {
    use super::*;

    #[tokio::test]
    async fn target_status_change_triggers_archive() {
        let f = Fixture::new("cross_target_archive").await;

        // seed a deployed deployment (activity=Deployed, target=Deployed)
        let seeded = models::Deployment {
            id: "dpl_1".to_string(),
            activity_status: DplActivity::Deployed,
            error_status: DplErrStatus::None,
            target_status: DplTarget::Deployed,
            config_instance_ids: vec!["cfg_inst_1".to_string()],
            ..Default::default()
        };
        f.deployment_stor
            .write("dpl_1".to_string(), seeded, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();

        // seed config instance content so content pull doesn't fail
        f.cfg_inst_content_stor
            .write(
                "cfg_inst_1".to_string(),
                "{}".to_string(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        // backend returns the same deployment but with target=Archived
        let backend_dep = make_archived_dpl("dpl_1", &["cfg_inst_1"]);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));

        f.sync().await.unwrap();

        // storage: deployment should be archived with no error
        let cached = read_deployment(&f.deployment_stor, "dpl_1").await;
        assert_eq!(cached.activity_status, DplActivity::Archived);
        assert_eq!(cached.target_status, DplTarget::Archived);
        assert_eq!(cached.error_status, DplErrStatus::None);

        // push: exactly one UpdateDeployment call
        assert_eq!(f.http_client.call_count(Call::UpdateDeployment), 1);

        // push body: activity=ARCHIVED, error=NONE
        let bodies = push_bodies(&f.http_client.requests());
        assert_eq!(bodies.len(), 1);
        assert_eq!(
            bodies[0].activity_status,
            Some(BackendActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_ARCHIVED)
        );
        assert_eq!(
            bodies[0].error_status,
            Some(BackendErrorStatus::DEPLOYMENT_ERROR_STATUS_NONE)
        );
    }

    #[tokio::test]
    async fn recovery_clears_error_status_and_pushes() {
        let f = Fixture::new("cross_recovery_push").await;

        // seed a retrying deployment (will recover on next deploy)
        let seeded = models::Deployment {
            id: "dpl_1".to_string(),
            activity_status: DplActivity::Queued,
            error_status: DplErrStatus::Retrying,
            target_status: DplTarget::Deployed,
            config_instance_ids: vec!["cfg_inst_1".to_string()],
            attempts: 3,
            // no cooldown (UNIX_EPOCH is in the past)
            ..Default::default()
        };
        f.deployment_stor
            .write("dpl_1".to_string(), seeded, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();

        // seed config instance content so deploy succeeds
        f.cfg_inst_content_stor
            .write(
                "cfg_inst_1".to_string(),
                "{}".to_string(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        // backend returns same deployment
        let backend_dep = make_deployment("dpl_1", &["cfg_inst_1"]);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));

        f.sync().await.unwrap();

        // storage: recovered — activity=Deployed, error=None, attempts=0
        let cached = read_deployment(&f.deployment_stor, "dpl_1").await;
        assert_eq!(cached.activity_status, DplActivity::Deployed);
        assert_eq!(cached.error_status, DplErrStatus::None);
        assert_eq!(cached.attempts, 0);

        // push: exactly one UpdateDeployment call
        assert_eq!(f.http_client.call_count(Call::UpdateDeployment), 1);

        // push body: activity=DEPLOYED, error=NONE
        let bodies = push_bodies(&f.http_client.requests());
        assert_eq!(bodies.len(), 1);
        assert_eq!(
            bodies[0].activity_status,
            Some(BackendActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DEPLOYED)
        );
        assert_eq!(
            bodies[0].error_status,
            Some(BackendErrorStatus::DEPLOYMENT_ERROR_STATUS_NONE)
        );
    }

    #[tokio::test]
    async fn apply_error_pushes_retrying_status() {
        let f = Fixture::new("cross_error_retrying_push").await;

        // backend returns a deployment with 1 config instance
        let backend_dep = make_deployment("dpl_1", &["cfg_inst_1"]);
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));

        // content fetch fails (non-network error, so no retry)
        f.http_client.set_get_config_instance_content(|_id| {
            Err(HTTPErr::MockErr(MockErr {
                is_network_conn_err: false,
            }))
        });

        f.sync().await.unwrap_err();

        // storage: error=Retrying, attempts > 0
        let cached = read_deployment(&f.deployment_stor, "dpl_1").await;
        assert_eq!(cached.error_status, DplErrStatus::Retrying);
        assert!(cached.attempts > 0, "attempts should be bumped");

        // push: exactly one UpdateDeployment call
        assert_eq!(f.http_client.call_count(Call::UpdateDeployment), 1);

        // push body: activity=QUEUED (unchanged), error=RETRYING
        let bodies = push_bodies(&f.http_client.requests());
        assert_eq!(bodies.len(), 1);
        assert_eq!(
            bodies[0].activity_status,
            Some(BackendActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_QUEUED)
        );
        assert_eq!(
            bodies[0].error_status,
            Some(BackendErrorStatus::DEPLOYMENT_ERROR_STATUS_RETRYING)
        );
    }
}

mod idempotency {
    use super::*;

    #[tokio::test]
    async fn double_sync_does_not_duplicate_stores() {
        let f = Fixture::new("sync_idempotent").await;
        let dpl = make_deployment_with_release("dpl_1", &["cfg_inst_1"], "rel_1", Some("gc_1"));
        f.http_client
            .set_list_all_deployments(move || Ok(vec![dpl.clone()]));
        f.http_client
            .set_get_config_instance_content(|_id| Ok("content".to_string()));

        // first sync
        f.sync().await.unwrap();
        assert_deployment_stored(&f.deployment_stor, "dpl_1").await;
        assert_cfg_inst_stored(&f.cfg_inst_stor, "cfg_inst_1").await;
        assert_release_stored(&f.release_stor, "rel_1").await;
        assert_git_commit_stored(&f.git_commit_stor, "gc_1").await;

        // second sync — same data
        f.sync().await.unwrap();

        // exactly one entry per store (no duplicates)
        let dpls = f.deployment_stor.values().await.unwrap();
        assert_eq!(dpls.len(), 1, "deployments should not duplicate");

        let cis = f.cfg_inst_stor.values().await.unwrap();
        assert_eq!(cis.len(), 1, "config instances should not duplicate");

        let rls = f.release_stor.values().await.unwrap();
        assert_eq!(rls.len(), 1, "releases should not duplicate");

        let gcs = f.git_commit_stor.values().await.unwrap();
        assert_eq!(gcs.len(), 1, "git commits should not duplicate");
    }
}
