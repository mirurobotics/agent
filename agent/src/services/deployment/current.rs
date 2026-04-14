// internal crates
use crate::models;
use crate::services::errors::ServiceErr;
use crate::storage;

pub async fn get_current(
    deployments: &storage::Deployments,
) -> Result<models::Deployment, ServiceErr> {
    let dpl = deployments
        .find_one("deployed", |d| {
            d.activity_status == models::DplActivity::Deployed
        })
        .await?;
    Ok(dpl)
}
