// Deployment-centric sync tests
// These replace the old config_instances sync tests.

use chrono::{DateTime, Utc};
use miru_agent::deploy::{apply, fsm};
use miru_agent::filesys::dir::Dir;
use miru_agent::filesys::Overwrite;
use miru_agent::http::errors::*;
use miru_agent::models::deployment::{DplActivity, DplErrStatus, DplTarget};
use miru_agent::storage::{self, CfgInstContent, CfgInsts, Deployments};
use miru_agent::sync::deployments::{sync, SyncArgs};
use miru_agent::sync::errors::SyncErr;

use crate::http::mock::{Call, MockClient};

use openapi_client::models::{
    Deployment as BackendDeployment, DeploymentActivityStatus as BackendActivityStatus,
    DeploymentErrorStatus as BackendErrorStatus, DeploymentStatus as BackendStatus,
    DeploymentTargetStatus as BackendTargetStatus,
};

// ========================= FIXTURE ========================= //

struct Fixture {
    deployment_stor: Deployments,
    cfg_inst_stor: CfgInsts,
    cfg_inst_content_stor: CfgInstContent,
    http_client: MockClient,
    staging_dir: Dir,
    target_dir: Dir,
    retry_policy: fsm::RetryPolicy,
    _dir: Dir,
}

impl Fixture {
    async fn new(name: &str) -> Self {
        let dir = Dir::create_temp_dir(name).await.unwrap();
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
        Self {
            deployment_stor,
            cfg_inst_stor,
            cfg_inst_content_stor,
            http_client: MockClient::default(),
            staging_dir: dir.subdir("staging"),
            target_dir: dir.subdir("deployments"),
            retry_policy: fsm::RetryPolicy::default(),
            _dir: dir,
        }
    }

    async fn sync(&self) -> Result<(), SyncErr> {
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
            },
            http_client: &self.http_client,
            opts: &opts,
            token: "test_token",
        })
        .await
    }
}

// ========================= FACTORIES ========================= //

fn make_backend_config_instance(
    id: &str,
    filepath: &str,
) -> openapi_client::models::ConfigInstance {
    openapi_client::models::ConfigInstance {
        object: openapi_client::models::config_instance::Object::ConfigInstance,
        id: id.to_string(),
        config_type_name: "test_type".to_string(),
        filepath: filepath.to_string(),
        created_at: "2024-01-01T00:00:00Z".to_string(),
        config_schema_id: "schema_1".to_string(),
        config_type_id: "ct_1".to_string(),
        config_type: None,
        content: None,
    }
}

fn make_backend_deployment_with_cis(id: &str, ci_ids: &[(&str, &str)]) -> BackendDeployment {
    let cis: Vec<_> = ci_ids
        .iter()
        .map(|(ci_id, filepath)| make_backend_config_instance(ci_id, filepath))
        .collect();
    BackendDeployment {
        object: openapi_client::models::deployment::Object::Deployment,
        id: id.to_string(),
        description: "test".to_string(),
        status: BackendStatus::DEPLOYMENT_STATUS_QUEUED,
        activity_status: BackendActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_QUEUED,
        error_status: BackendErrorStatus::DEPLOYMENT_ERROR_STATUS_NONE,
        target_status: BackendTargetStatus::DEPLOYMENT_TARGET_STATUS_DEPLOYED,
        device_id: "dev_1".to_string(),
        release_id: "rls_1".to_string(),
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
        release: None,
        config_instances: Some(cis),
    }
}

fn make_backend_deployment(id: &str) -> BackendDeployment {
    let ci_id = format!("{id}_ci_1");
    make_backend_deployment_with_cis(id, &[(&ci_id, "test/config.json")])
}

// ========================= TESTS ========================= //

#[tokio::test]
async fn empty_sync_succeeds() {
    let f = Fixture::new("sync_empty").await;
    assert!(f.sync().await.is_ok());
}

mod pull {
    use super::*;

    #[tokio::test]
    async fn stores_deployments_and_config_instances() {
        let f = Fixture::new("sync_pull").await;
        let backend_dep = make_backend_deployment("dpl_1");
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));

        let _ = f.sync().await;

        // Deployment stored with correct post-apply state
        // FSM: target=Deployed, activity=Queued -> Deploy -> Deployed
        let cached = f
            .deployment_stor
            .read_optional("dpl_1".to_string())
            .await
            .unwrap()
            .expect("deployment should be stored");
        assert_eq!(cached.id, "dpl_1");
        assert_eq!(cached.activity_status, DplActivity::Deployed);
        assert_eq!(cached.target_status, DplTarget::Deployed);
        assert_eq!(cached.config_instance_ids, vec!["dpl_1_ci_1"]);

        // Config instance metadata stored
        let cfg_inst = f
            .cfg_inst_stor
            .read_optional("dpl_1_ci_1".to_string())
            .await
            .unwrap()
            .expect("config instance should be stored");
        assert_eq!(cfg_inst.filepath, "test/config.json");
    }

    #[tokio::test]
    async fn failure_returns_error() {
        let f = Fixture::new("sync_fail").await;
        f.http_client.set_list_all_deployments(|| {
            Err(HTTPErr::MockErr(MockErr {
                is_network_conn_err: true,
            }))
        });

        assert!(f.sync().await.is_err());
    }

    #[tokio::test]
    async fn merge_preserves_agent_fields() {
        let f = Fixture::new("sync_merge").await;

        // Seed a deployment with agent-side fields set
        let future_cooldown = Utc::now() + chrono::TimeDelta::seconds(3600);
        let seeded = miru_agent::models::deployment::Deployment {
            id: "dpl_1".to_string(),
            description: "old description".to_string(),
            activity_status: DplActivity::Deployed,
            error_status: DplErrStatus::None,
            target_status: DplTarget::Deployed,
            device_id: "dev_1".to_string(),
            release_id: "rls_1".to_string(),
            created_at: DateTime::<Utc>::UNIX_EPOCH,
            updated_at: DateTime::<Utc>::UNIX_EPOCH,
            config_instance_ids: vec!["dpl_1_ci_1".to_string()],
            attempts: 5,
            cooldown_ends_at: future_cooldown,
        };
        f.deployment_stor
            .write("dpl_1".to_string(), seeded, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();

        // Backend returns same ID with updated description, activity=Deployed
        // FSM sees Wait (cooldown) so agent fields stay untouched
        let mut backend_dep = make_backend_deployment("dpl_1");
        backend_dep.description = "new description".to_string();
        backend_dep.activity_status = BackendActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DEPLOYED;
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));

        let _ = f.sync().await;

        let cached = f
            .deployment_stor
            .read_optional("dpl_1".to_string())
            .await
            .unwrap()
            .expect("deployment should be stored");

        // Backend fields updated
        assert_eq!(cached.description, "new description");
        assert_eq!(cached.activity_status, DplActivity::Deployed);

        // Agent-side fields preserved
        assert_eq!(cached.attempts, 5);
        assert_eq!(cached.cooldown_ends_at, future_cooldown);
    }

    #[tokio::test]
    async fn fetches_content_individually() {
        let f = Fixture::new("sync_content_individual").await;
        let backend_dep = make_backend_deployment_with_cis(
            "dpl_1",
            &[("ci_1", "config_a.json"), ("ci_2", "config_b.yaml")],
        );
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));
        f.http_client
            .set_get_config_instance_content(|id| match id {
                "ci_1" => Ok(r#"{"alpha": true}"#.to_string()),
                "ci_2" => Ok("beta: yes".to_string()),
                other => panic!("unexpected config instance id: {other}"),
            });

        let _ = f.sync().await;

        let content_1 = f
            .cfg_inst_content_stor
            .read_optional("ci_1".to_string())
            .await
            .unwrap();
        assert_eq!(content_1.as_deref(), Some(r#"{"alpha": true}"#));

        let content_2 = f
            .cfg_inst_content_stor
            .read_optional("ci_2".to_string())
            .await
            .unwrap();
        assert_eq!(content_2.as_deref(), Some("beta: yes"));

        assert_eq!(f.http_client.call_count(Call::GetConfigInstanceContent), 2);
    }

    #[tokio::test]
    async fn content_fetch_failure_continues() {
        let f = Fixture::new("sync_content_partial").await;
        let backend_dep = make_backend_deployment_with_cis(
            "dpl_1",
            &[("ci_1", "config_a.json"), ("ci_2", "config_b.yaml")],
        );
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));
        f.http_client
            .set_get_config_instance_content(|id| match id {
                "ci_1" => Err(HTTPErr::MockErr(MockErr {
                    is_network_conn_err: false,
                })),
                "ci_2" => Ok("success content".to_string()),
                other => panic!("unexpected config instance id: {other}"),
            });

        let _ = f.sync().await;

        // Failed fetch — not stored
        let content_1 = f
            .cfg_inst_content_stor
            .read_optional("ci_1".to_string())
            .await
            .unwrap();
        assert!(content_1.is_none(), "failed content should not be stored");

        // Successful fetch — stored
        let content_2 = f
            .cfg_inst_content_stor
            .read_optional("ci_2".to_string())
            .await
            .unwrap();
        assert_eq!(content_2.as_deref(), Some("success content"));
    }
}

mod push {
    use super::*;

    #[tokio::test]
    async fn sends_dirty_deployments() {
        let f = Fixture::new("sync_push").await;
        let backend_dep = make_backend_deployment("dpl_1");
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));

        let _ = f.sync().await;

        assert!(
            f.http_client.call_count(Call::UpdateDeployment) > 0,
            "push should send UpdateDeployment for dirty deployments"
        );
    }

    #[tokio::test]
    async fn error_accumulation() {
        let f = Fixture::new("sync_push_err").await;
        let backend_dep = make_backend_deployment("dpl_1");
        f.http_client
            .set_list_all_deployments(move || Ok(vec![backend_dep.clone()]));
        f.http_client.set_update_deployment(|| {
            Err(HTTPErr::MockErr(MockErr {
                is_network_conn_err: false,
            }))
        });

        let result = f.sync().await;

        assert!(result.is_err(), "sync should fail when push fails");
        let err = result.unwrap_err();
        assert!(
            matches!(err, SyncErr::SyncErrors(_)),
            "error should be SyncErrors, got: {err:?}"
        );
    }
}
