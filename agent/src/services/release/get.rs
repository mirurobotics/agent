// internal crates
use crate::filesys;
use crate::models;
use crate::services::{backend::BackendFetcher, errors::ServiceErr};
use crate::storage;

// external crates
use tracing::error;

pub async fn get<B: BackendFetcher>(
    releases: &storage::Releases,
    backend: &B,
    id: String,
) -> Result<models::Release, ServiceErr> {
    let cached = releases.read_optional(id.clone()).await?;
    if let Some(rls) = cached {
        return Ok(rls);
    }
    let backend_rls = backend.fetch_release(&id).await?;
    let storage_rls = models::Release::from(backend_rls);
    cache_release(releases, storage_rls.clone()).await;
    Ok(storage_rls)
}

async fn cache_release(releases: &storage::Releases, storage_rls: models::Release) {
    let id = storage_rls.id.clone();
    if let Err(e) = releases
        .write(
            id.clone(),
            storage_rls,
            |_, _| false,
            filesys::Overwrite::Allow,
        )
        .await
    {
        error!("failed to cache release {id}: {e}");
    }
}
