use miru_agent::filesys::{self, Overwrite};
use miru_agent::models::{Deployment, DplActivity, DplErrStatus, DplTarget};
use miru_agent::services::deployment as dpl_svc;
use miru_agent::services::ServiceErr;
use miru_agent::storage::Deployments;

use chrono::{DateTime, Utc};

async fn setup(name: &str) -> (filesys::Dir, Deployments) {
    let dir = filesys::Dir::create_temp_dir(name).await.unwrap();
    let (stor, _) = Deployments::spawn(16, dir.file("deployments.json"), 1000)
        .await
        .unwrap();
    (dir, stor)
}

fn make_deployment(id: &str, activity: DplActivity) -> Deployment {
    Deployment {
        id: id.to_string(),
        activity_status: activity,
        error_status: DplErrStatus::None,
        target_status: DplTarget::Deployed,
        created_at: DateTime::<Utc>::UNIX_EPOCH,
        ..Default::default()
    }
}

pub mod get_deployment {
    use super::*;

    #[tokio::test]
    async fn returns_deployment_by_id() {
        let (_dir, stor) = setup("get_dpl_by_id").await;
        let dpl = make_deployment("dpl_1", DplActivity::Deployed);
        stor.write(
            "dpl_1".to_string(),
            dpl.clone(),
            |_, _| false,
            Overwrite::Allow,
        )
        .await
        .unwrap();

        let result = dpl_svc::get(&stor, "dpl_1".to_string()).await.unwrap();
        assert_eq!(result.id, "dpl_1");
        assert_eq!(result.activity_status, DplActivity::Deployed);
    }

    #[tokio::test]
    async fn not_found_returns_error() {
        let (_dir, stor) = setup("get_dpl_not_found").await;

        let result = dpl_svc::get(&stor, "nonexistent".to_string()).await;
        assert!(matches!(result, Err(ServiceErr::CacheErr(_))));
    }
}

pub mod get_current_deployment {
    use super::*;

    #[tokio::test]
    async fn returns_deployed_deployment() {
        let (_dir, stor) = setup("get_cur_dpl").await;
        let dpl = make_deployment("dpl_1", DplActivity::Deployed);
        stor.write(
            "dpl_1".to_string(),
            dpl.clone(),
            |_, _| false,
            Overwrite::Allow,
        )
        .await
        .unwrap();

        let result = dpl_svc::get_current(&stor).await.unwrap();
        assert_eq!(result.id, "dpl_1");
        assert_eq!(result.activity_status, DplActivity::Deployed);
    }

    #[tokio::test]
    async fn skips_non_deployed() {
        let (_dir, stor) = setup("get_cur_dpl_skip").await;
        let queued = make_deployment("dpl_q", DplActivity::Queued);
        stor.write("dpl_q".to_string(), queued, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();
        let deployed = make_deployment("dpl_d", DplActivity::Deployed);
        stor.write(
            "dpl_d".to_string(),
            deployed,
            |_, _| false,
            Overwrite::Allow,
        )
        .await
        .unwrap();

        let result = dpl_svc::get_current(&stor).await.unwrap();
        assert_eq!(result.id, "dpl_d");
    }

    #[tokio::test]
    async fn no_deployed_returns_error() {
        let (_dir, stor) = setup("get_cur_dpl_none").await;
        let queued = make_deployment("dpl_q", DplActivity::Queued);
        stor.write("dpl_q".to_string(), queued, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();

        let result = dpl_svc::get_current(&stor).await;
        assert!(matches!(result, Err(ServiceErr::CacheErr(_))));
    }

    #[tokio::test]
    async fn empty_cache_returns_error() {
        let (_dir, stor) = setup("get_cur_dpl_empty").await;

        let result = dpl_svc::get_current(&stor).await;
        assert!(matches!(result, Err(ServiceErr::CacheErr(_))));
    }

    #[tokio::test]
    async fn multiple_deployed_returns_error() {
        let (_dir, stor) = setup("get_cur_dpl_multi").await;
        let dpl_a = make_deployment("dpl_a", DplActivity::Deployed);
        stor.write("dpl_a".to_string(), dpl_a, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();
        let dpl_b = make_deployment("dpl_b", DplActivity::Deployed);
        stor.write("dpl_b".to_string(), dpl_b, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();

        let result = dpl_svc::get_current(&stor).await;
        assert!(matches!(result, Err(ServiceErr::CacheErr(_))));
    }
}
