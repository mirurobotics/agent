// internal crates
use crate::filesys::Overwrite;
use crate::models;
use crate::services::{backend::BackendFetcher, errors::ServiceErr};
use crate::storage;
use crate::sync;

pub async fn get<B: BackendFetcher>(
    deployments: &storage::Deployments,
    backend: &B,
    id: String,
) -> Result<models::Deployment, ServiceErr> {
    let cached = deployments.read_optional(id.clone()).await?;
    if let Some(dpl) = cached {
        return Ok(dpl);
    }
    let backend_dpl = backend.fetch_deployment(&id).await?;

    let cfg_insts = backend_dpl.config_instances.as_ref().ok_or_else(|| {
        ServiceErr::SyncErr(sync::SyncErr::CfgInstsNotExpanded(
            sync::errors::CfgInstsNotExpandedErr {
                deployment_id: backend_dpl.id.clone(),
            },
        ))
    })?;
    let cfg_inst_ids: Vec<String> = cfg_insts.iter().map(|ci| ci.id.clone()).collect();

    let storage_dpl = models::Deployment::from_backend(backend_dpl, cfg_inst_ids);
    cache_deployment(deployments, storage_dpl.clone()).await;
    Ok(storage_dpl)
}

async fn cache_deployment(deployments: &storage::Deployments, storage_dpl: models::Deployment) {
    let id = storage_dpl.id.clone();
    if let Err(e) = deployments
        .write(
            id.clone(),
            storage_dpl,
            |old, _| old.is_some_and(|e| e.is_dirty),
            Overwrite::Allow,
        )
        .await
    {
        tracing::error!(
            error = ?e,
            id = %id,
            "failed to cache fetched deployment; returning value anyway"
        );
    }
}
