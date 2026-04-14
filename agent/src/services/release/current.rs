// internal crates
use crate::models;
use crate::services::{
    backend::BackendFetcher, deployment as dpl_svc, errors::ServiceErr, release as rls_svc,
};
use crate::storage;

pub async fn get_current(
    deployments: &storage::Deployments,
    releases: &storage::Releases,
    backend: &impl BackendFetcher,
) -> Result<models::Release, ServiceErr> {
    let dpl = dpl_svc::get_current(deployments).await?;
    let release = rls_svc::get(releases, backend, dpl.release_id).await?;
    Ok(release)
}
