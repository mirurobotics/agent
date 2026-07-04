// internal crates
use crate::disk;
use crate::models;
use crate::services::{
    backend::BackendFetcher, deployment as dpl_svc, errors::ServiceErr, release as rls_svc,
};

pub async fn get_current(
    deployments: &disk::Deployments,
    releases: &disk::Releases,
    backend: &impl BackendFetcher,
) -> Result<models::Release, ServiceErr> {
    let dpl = dpl_svc::get_current(deployments).await?;
    let release = rls_svc::get(releases, backend, dpl.release_id).await?;
    Ok(release)
}
