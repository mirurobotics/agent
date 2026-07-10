// internal crates
use crate::cache;
use crate::models;

pub type DplEntry = cache::CacheEntry<models::DeploymentID, models::Deployment>;
pub type Deployments = cache::FileCache<models::DeploymentID, models::Deployment>;

pub fn is_dirty(old: Option<&DplEntry>, new: &models::Deployment) -> bool {
    let old = match old {
        Some(old) => old,
        None => return true,
    };
    old.is_dirty
        || old.value.activity_status != new.activity_status
        || old.value.error_status != new.error_status
        || old.value.deployed_at != new.deployed_at
        || old.value.archived_at != new.archived_at
}

/// Find the currently Deployed deployment, if any.
pub async fn find_deployed(
    deployments: &Deployments,
) -> Result<Option<models::Deployment>, super::DiskErr> {
    let dpl = deployments
        .find_one_optional("deployed", |d| {
            d.activity_status == models::DplActivity::Deployed
        })
        .await?;
    Ok(dpl)
}

#[cfg(test)]
mod tests {
    // internal crates
    use crate::disk::{self, Layout};
    use crate::filesys;
    use crate::models::{Deployment, DplActivity};

    // =============================== TEST HELPERS ================================= //

    async fn deployments() -> (filesys::dirs::TempDir, disk::Deployments) {
        let dir = filesys::dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        let (deployments, _) = disk::Deployments::spawn(64, layout.deployments(), 1000)
            .await
            .unwrap();
        (dir, deployments)
    }

    // A Deployed deployment is returned by find_deployed.
    #[tokio::test]
    async fn find_deployed_returns_deployed() {
        let (_dir, deployments) = deployments().await;
        deployments
            .write_if_absent(
                "dpl".to_string(),
                Deployment {
                    id: "dpl".to_string(),
                    activity_status: DplActivity::Deployed,
                    release_id: "rel".to_string(),
                    ..Default::default()
                },
                |_, _| false,
            )
            .await
            .unwrap();

        let deployed = super::find_deployed(&deployments).await.unwrap();
        let deployed = deployed.expect("expected a deployed deployment");
        assert_eq!(deployed.id, "dpl");
        assert_eq!(deployed.release_id, "rel");
    }

    // With only a Queued deployment, find_deployed returns None.
    #[tokio::test]
    async fn find_deployed_none_when_only_queued() {
        let (_dir, deployments) = deployments().await;
        deployments
            .write_if_absent(
                "dpl".to_string(),
                Deployment {
                    id: "dpl".to_string(),
                    activity_status: DplActivity::Queued,
                    release_id: "rel".to_string(),
                    ..Default::default()
                },
                |_, _| false,
            )
            .await
            .unwrap();

        assert!(super::find_deployed(&deployments).await.unwrap().is_none());
    }

    // A shut-down deployments store makes find_deployed return an error (it is NOT
    // swallowed to None as the old union-based traversal assumed).
    #[tokio::test]
    async fn find_deployed_errors_when_store_shut_down() {
        let (_dir, deployments) = deployments().await;
        deployments.shutdown().await.unwrap();

        let err = super::find_deployed(&deployments).await.unwrap_err();
        assert!(matches!(err, crate::disk::DiskErr::CacheErr(_)));
    }
}
