use miru_agent::filesys::dir::Dir;
use miru_agent::filesys::Overwrite;
use miru_agent::models::deployment::{DplActivity, DplErrStatus, DplTarget};
use miru_agent::models::{Deployment, Release};
use miru_agent::services::errors::ServiceErr;
use miru_agent::services::release as rls_svc;
use miru_agent::storage::{Deployments, Releases};

use chrono::{DateTime, Utc};

async fn setup(name: &str) -> (Dir, Deployments, Releases) {
    let dir = Dir::create_temp_dir(name).await.unwrap();
    let (dpl_stor, _) = Deployments::spawn(16, dir.file("deployments.json"), 1000)
        .await
        .unwrap();
    let (rls_stor, _) = Releases::spawn(16, dir.file("releases.json"), 1000)
        .await
        .unwrap();
    (dir, dpl_stor, rls_stor)
}

pub mod get_release {
    use super::*;

    #[tokio::test]
    async fn returns_release_by_id() {
        let (_dir, _, rls_stor) = setup("get_rls_by_id").await;
        let rls = Release {
            id: "rls_1".to_string(),
            version: "1.0.0".to_string(),
            ..Default::default()
        };
        rls_stor
            .write(
                "rls_1".to_string(),
                rls.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        let result = rls_svc::get(&rls_stor, "rls_1".to_string()).await.unwrap();
        assert_eq!(result.id, "rls_1");
        assert_eq!(result.version, "1.0.0");
    }

    #[tokio::test]
    async fn not_found_returns_error() {
        let (_dir, _, rls_stor) = setup("get_rls_not_found").await;

        let result = rls_svc::get(&rls_stor, "nonexistent".to_string()).await;
        assert!(matches!(result, Err(ServiceErr::CacheErr(_))));
    }
}

pub mod get_current_release {
    use super::*;

    #[tokio::test]
    async fn returns_release_for_deployed_deployment() {
        let (_dir, dpl_stor, rls_stor) = setup("get_cur_rls").await;

        let rls = Release {
            id: "rls_1".to_string(),
            version: "2.0.0".to_string(),
            ..Default::default()
        };
        rls_stor
            .write("rls_1".to_string(), rls, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();

        let dpl = Deployment {
            id: "dpl_1".to_string(),
            release_id: "rls_1".to_string(),
            activity_status: DplActivity::Deployed,
            error_status: DplErrStatus::None,
            target_status: DplTarget::Deployed,
            created_at: DateTime::<Utc>::UNIX_EPOCH,
            ..Default::default()
        };
        dpl_stor
            .write("dpl_1".to_string(), dpl, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();

        let result = rls_svc::get_current(&dpl_stor, &rls_stor).await.unwrap();
        assert_eq!(result.id, "rls_1");
        assert_eq!(result.version, "2.0.0");
    }

    #[tokio::test]
    async fn no_deployed_deployment_returns_error() {
        let (_dir, dpl_stor, rls_stor) = setup("get_cur_rls_no_dpl").await;

        let result = rls_svc::get_current(&dpl_stor, &rls_stor).await;
        assert!(matches!(result, Err(ServiceErr::CacheErr(_))));
    }

    #[tokio::test]
    async fn deployed_deployment_with_missing_release_returns_error() {
        let (_dir, dpl_stor, rls_stor) = setup("get_cur_rls_missing").await;

        let dpl = Deployment {
            id: "dpl_1".to_string(),
            release_id: "rls_missing".to_string(),
            activity_status: DplActivity::Deployed,
            error_status: DplErrStatus::None,
            target_status: DplTarget::Deployed,
            created_at: DateTime::<Utc>::UNIX_EPOCH,
            ..Default::default()
        };
        dpl_stor
            .write("dpl_1".to_string(), dpl, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();

        let result = rls_svc::get_current(&dpl_stor, &rls_stor).await;
        assert!(matches!(result, Err(ServiceErr::CacheErr(_))));
    }

    #[tokio::test]
    async fn multiple_deployed_deployments_returns_error() {
        let (_dir, dpl_stor, rls_stor) = setup("get_cur_rls_multi_dpl").await;

        let dpl_a = Deployment {
            id: "dpl_a".to_string(),
            release_id: "rls_1".to_string(),
            activity_status: DplActivity::Deployed,
            error_status: DplErrStatus::None,
            target_status: DplTarget::Deployed,
            created_at: DateTime::<Utc>::UNIX_EPOCH,
            ..Default::default()
        };
        dpl_stor
            .write("dpl_a".to_string(), dpl_a, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();

        let dpl_b = Deployment {
            id: "dpl_b".to_string(),
            release_id: "rls_1".to_string(),
            activity_status: DplActivity::Deployed,
            error_status: DplErrStatus::None,
            target_status: DplTarget::Deployed,
            created_at: DateTime::<Utc>::UNIX_EPOCH,
            ..Default::default()
        };
        dpl_stor
            .write("dpl_b".to_string(), dpl_b, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();

        let result = rls_svc::get_current(&dpl_stor, &rls_stor).await;
        assert!(matches!(result, Err(ServiceErr::CacheErr(_))));
    }
}
