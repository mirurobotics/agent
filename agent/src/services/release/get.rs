// internal crates
use crate::models;
use crate::services::{deployment as dpl_svc, errors::ServiceErr};
use crate::storage;

pub async fn get(releases: &storage::Releases, id: String) -> Result<models::Release, ServiceErr> {
    let release = releases.read(id).await?;
    Ok(release)
}

pub async fn get_current(
    deployments: &storage::Deployments,
    releases: &storage::Releases,
) -> Result<models::Release, ServiceErr> {
    let dpl = dpl_svc::get_current(deployments).await?;
    let release = releases.read(dpl.release_id).await?;
    Ok(release)
}
