// internal crates
use crate::disk;
use crate::models;
use crate::services::errors::ServiceErr;

pub async fn get_current(
    deployments: &disk::Deployments,
) -> Result<models::Deployment, ServiceErr> {
    let dpl = deployments
        .find_one("deployed", |d| {
            d.activity_status == models::DplActivity::Deployed
        })
        .await?;
    Ok(dpl)
}
