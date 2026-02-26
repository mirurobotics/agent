use crate::models::release::Release;
use crate::services::deployment as dpl_svc;
use crate::services::errors::ServiceErr;
use crate::storage::{Deployments, Releases};

pub async fn get(releases: &Releases, id: String) -> Result<Release, ServiceErr> {
    let release = releases.read(id).await?;
    Ok(release)
}

pub async fn get_current(
    deployments: &Deployments,
    releases: &Releases,
) -> Result<Release, ServiceErr> {
    let dpl = dpl_svc::get_current(deployments).await?;
    let release = releases.read(dpl.release_id).await?;
    Ok(release)
}
