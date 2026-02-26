use crate::models::deployment::{Deployment, DplActivity};
use crate::services::errors::ServiceErr;
use crate::storage::Deployments;

pub async fn get(deployments: &Deployments, id: String) -> Result<Deployment, ServiceErr> {
    let dpl = deployments.read(id).await?;
    Ok(dpl)
}

pub async fn get_current(deployments: &Deployments) -> Result<Deployment, ServiceErr> {
    let dpl = deployments
        .find_one("deployed", |d| d.activity_status == DplActivity::Deployed)
        .await?;
    Ok(dpl)
}
