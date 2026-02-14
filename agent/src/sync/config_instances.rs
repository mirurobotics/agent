use crate::crud::prelude::*;
use crate::deploy::{apply::apply, fsm};
use crate::filesys::dir::Dir;
// TODO: Refactor to use DeploymentsExt instead of ConfigInstancesExt
// Config instances are now accessed through deployments with expand=config_instances
use crate::http::search::SearchOperator;
use crate::models::config_instance::{ActivityStatus, ConfigInstance, ErrorStatus, TargetStatus};
use crate::storage::config_instances::{ConfigInstanceCache, ConfigInstanceContentCache};
use crate::sync::errors::{
    ConfigInstanceContentNotFoundErr, MissingExpandedInstancesErr, SyncErr, SyncErrors,
};
use crate::trace;
use openapi_client::models::{
    ConfigInstance as BackendConfigInstance, ConfigInstanceActivityStatus, ConfigInstanceExpand,
    UpdateConfigInstanceRequest,
};

// external crates
use tracing::{debug, error};

// =================================== SYNC ======================================== //
// TODO: Refactor to use DeploymentsExt - config instances are now accessed through deployments
pub async fn sync<HTTPClientT>(
    cfg_inst_cache: &ConfigInstanceCache,
    cfg_inst_content_cache: &ConfigInstanceContentCache,
    http_client: &HTTPClientT,
    device_id: &str,
    deployment_dir: &Dir,
    fsm_settings: &fsm::Settings,
    token: &str,
) -> Result<(), SyncErr> {
    let mut errors = Vec::new();

    // pull config instances from server
    debug!("Pulling config instances from server");
    let result = pull(
        cfg_inst_cache,
        cfg_inst_content_cache,
        http_client,
        device_id,
        token,
    )
    .await;
    match result {
        Ok(_) => (),
        Err(e) => {
            errors.push(e);
        }
    };

    // read the config instances which need to be applied
    debug!("Reading config instances which need to be applied");
    let cfg_insts_to_apply = cfg_inst_cache
        .find_where(|cfg_inst| fsm::is_action_required(fsm::next_action(cfg_inst, true)))
        .await?;
    let cfg_insts_to_apply = cfg_insts_to_apply
        .into_iter()
        .map(|cfg_inst| (cfg_inst.id.clone(), cfg_inst))
        .collect();

    // apply deployments
    apply(
        cfg_insts_to_apply,
        cfg_inst_cache,
        cfg_inst_content_cache,
        deployment_dir,
        fsm_settings,
    )
    .await?;

    // push config instances to server
    debug!("Pushing config instances to server");
    let result = push(cfg_inst_cache, http_client, token).await;
    match result {
        Ok(_) => (),
        Err(e) => {
            errors.push(e);
        }
    };

    if errors.is_empty() {
        Ok(())
    } else {
        Err(SyncErr::SyncErrors(SyncErrors {
            errors,
            trace: trace!(),
        }))
    }
}

// =================================== PULL ======================================== //
pub async fn pull<HTTPClientT>(
    cfg_inst_cache: &ConfigInstanceCache,
    cfg_inst_content_cache: &ConfigInstanceContentCache,
    http_client: &HTTPClientT,
    device_id: &str,
    token: &str,
) -> Result<(), SyncErr> {
    let active_insts = fetch_active_cfg_insts(http_client, device_id, token).await?;
    debug!(
        "Found {} active config instances: {:?}",
        active_insts.len(),
        active_insts
    );

    let categorized_cfg_insts = categorize_cfg_insts(cfg_inst_cache, active_insts).await?;
    debug!(
        "Found {} unknown config instances: {:?}",
        categorized_cfg_insts.unknown.len(),
        categorized_cfg_insts.unknown
    );
    debug!(
        "Found {} instances with updated target status: {:?}",
        categorized_cfg_insts.update_target_status.len(),
        categorized_cfg_insts.update_target_status
    );

    let unknown_cfg_insts = fetch_cfg_insts_with_content(
        http_client,
        device_id,
        categorized_cfg_insts
            .unknown
            .iter()
            .map(|inst| inst.id.clone())
            .collect(),
        token,
    )
    .await?;

    debug!(
        "Adding {} unknown instances to storage",
        unknown_cfg_insts.len()
    );
    add_unknown_cfg_insts_to_storage(cfg_inst_cache, cfg_inst_content_cache, unknown_cfg_insts)
        .await?;

    debug!(
        "Updating target status for {} instances",
        categorized_cfg_insts.update_target_status.len()
    );
    update_target_status_instances(cfg_inst_cache, categorized_cfg_insts.update_target_status)
        .await?;

    Ok(())
}

async fn fetch_active_cfg_insts<HTTPClientT>(
    http_client: &HTTPClientT,
    device_id: &str,
    token: &str,
) -> Result<Vec<BackendConfigInstance>, SyncErr> {
    let filters = ConfigInstanceFiltersBuilder::new(device_id.to_string())
        .with_activity_status_filter(ActivityStatusFilter {
            negate: false,
            op: SearchOperator::Equals,
            // we don't want to fetch 'created' or 'validating' activity statuses
            // because created instances because they have not cleared the validation
            // stage and are thus not ready for deployment. We don't want to fetch the
            // 'removed' status because they're already removed and therefore useless to
            // us (there's also a lot of them so it's not very performant to fetch them)
            val: vec![
                ConfigInstanceActivityStatus::CONFIG_INSTANCE_ACTIVITY_STATUS_QUEUED,
                ConfigInstanceActivityStatus::CONFIG_INSTANCE_ACTIVITY_STATUS_DEPLOYED,
            ],
        })
        .build();
    http_client
        .list_all_config_instances(filters, &[] as &[ConfigInstanceExpand], token)
        .await
        .map_err(SyncErr::from)
}

#[derive(Debug)]
pub struct CategorizedConfigInstances {
    pub unknown: Vec<BackendConfigInstance>,
    pub update_target_status: Vec<ConfigInstance>,
    pub other: Vec<BackendConfigInstance>,
}

async fn categorize_cfg_insts(
    cfg_inst_cache: &ConfigInstanceCache,
    active_cfgs_insts: Vec<BackendConfigInstance>,
) -> Result<CategorizedConfigInstances, SyncErr> {
    let mut categorized = CategorizedConfigInstances {
        unknown: Vec::new(),
        update_target_status: Vec::new(),
        other: Vec::new(),
    };

    // deleteme
    match cfg_inst_cache.entries().await {
        Ok(entries) => {
            debug!("Found {} instances in cache: {:?}", entries.len(), entries);
        }
        Err(e) => {
            error!("Failed to read config instances from cache: {}", e);
        }
    }

    // unknown config instances
    for server_inst in active_cfgs_insts {
        // check if the config instance is known
        let mut storage_inst = match cfg_inst_cache
            .read_optional(server_inst.id.clone())
            .await? {
            Some(storage_inst) => {
                debug!("Found config instance {}in cache", storage_inst.id);
                storage_inst
            }
            None => {
                debug!("Config instance {} not found in cache", server_inst.id);
                categorized.unknown.push(server_inst);
                continue;
            }
        };

        // check if the target status matches
        if storage_inst.target_status != TargetStatus::from_backend(&server_inst.target_status) {
            debug!(
                "Config instance {} has updated target status",
                storage_inst.id
            );
            storage_inst.target_status = TargetStatus::from_backend(&server_inst.target_status);
            categorized.update_target_status.push(storage_inst);
        } else {
            debug!(
                "Config instance {} has the same target status",
                storage_inst.id
            );
            categorized.other.push(server_inst);
        }
    }

    Ok(categorized)
}

async fn fetch_cfg_insts_with_content<HTTPClientT>(
    http_client: &HTTPClientT,
    device_id: &str,
    ids: Vec<String>,
    token: &str,
) -> Result<Vec<BackendConfigInstance>, SyncErr> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    // read the unknown config instances from the server with config instance content expanded
    let filters = ConfigInstanceFiltersBuilder::new(device_id.to_string())
        .with_id_filter(IDFilter {
            negate: false,
            op: SearchOperator::Equals,
            val: ids.clone(),
        })
        .build();
    let cfg_insts = http_client
        .list_all_config_instances(
            filters,
            [ConfigInstanceExpand::CONFIG_INSTANCE_EXPAND_CONTENT],
            token,
        )
        .await?;

    if cfg_insts.len() != ids.len() {
        return Err(SyncErr::MissingExpandedInstancesErr(
            MissingExpandedInstancesErr {
                expected_ids: ids,
                actual_ids: cfg_insts.iter().map(|inst| inst.id.clone()).collect(),
                trace: trace!(),
            },
        ));
    }

    Ok(cfg_insts)
}

async fn add_unknown_cfg_insts_to_storage(
    cfg_inst_cache: &ConfigInstanceCache,
    cfg_inst_content_cache: &ConfigInstanceContentCache,
    unknown_insts: Vec<BackendConfigInstance>,
) -> Result<(), SyncErr> {
    // add the unknown config instances to the cache
    for mut unknown_inst in unknown_insts {
        // throw an error since if the config instance content isn't expanded for this
        // one it won't be expanded for any others and none of the config instances will
        // therefore be added to the cache
        let cfg_inst_content = match unknown_inst.content {
            Some(cfg_inst_content) => cfg_inst_content,
            None => {
                return Err(SyncErr::ConfigInstanceContentNotFound(
                    ConfigInstanceContentNotFoundErr {
                        cfg_inst_id: unknown_inst.id.clone(),
                        trace: trace!(),
                    },
                ));
            }
        };
        unknown_inst.content = None;

        let overwrite = true;
        if let Err(e) = cfg_inst_content_cache
            .write(
                unknown_inst.id.clone(),
                cfg_inst_content,
                |_, _| false,
                overwrite,
            )
            .await
        {
            error!(
                "Failed to write config instance '{}' content to cache: {}",
                unknown_inst.id, e
            );
            continue;
        }

        let unknown_inst_id = unknown_inst.id.clone();
        let storage_inst = ConfigInstance::from_backend(unknown_inst);
        let overwrite = true;
        if let Err(e) = cfg_inst_cache
            .write(
                unknown_inst_id.clone(),
                storage_inst,
                |_, _| false,
                overwrite,
            )
            .await
        {
            error!(
                "Failed to write config instance '{}' to cache: {}",
                unknown_inst_id, e
            );
            continue;
        }
    }
    Ok(())
}

async fn update_target_status_instances(
    cfg_inst_cache: &ConfigInstanceCache,
    update_target_status: Vec<ConfigInstance>,
) -> Result<(), SyncErr> {
    for cfg_inst in update_target_status {
        let cfg_inst_id = cfg_inst.id.clone();

        // read the config instance from the cache to update only select fields
        let cache_inst = match cfg_inst_cache.read(cfg_inst_id.clone()).await {
            Ok(cache_inst) => cache_inst,
            Err(e) => {
                error!(
                    "Failed to read config instance '{}' from cache: {}",
                    cfg_inst_id, e
                );
                continue;
            }
        };
        let updated_inst = ConfigInstance {
            target_status: cfg_inst.target_status,
            updated_at: cfg_inst.updated_at,
            ..cache_inst
        };

        // write the updated config instance to the cache
        let overwrite = true;
        if let Err(e) = cfg_inst_cache
            .write(cfg_inst_id.clone(), updated_inst, |_, _| false, overwrite)
            .await
        {
            error!(
                "Failed to write config instance '{}' to cache: {}",
                cfg_inst_id, e
            );
            continue;
        }
    }

    Ok(())
}

// =================================== PUSH ======================================== //
pub async fn push<HTTPClientT>(
    cfg_inst_cache: &ConfigInstanceCache,
    http_client: &HTTPClientT,
    token: &str,
) -> Result<(), SyncErr> {
    // get all unsynced config instances
    let unsynced_cfg_insts = cfg_inst_cache
        .get_dirty_entries()
        .await?;
    debug!(
        "Found {} unsynced config instances: {:?}",
        unsynced_cfg_insts.len(),
        unsynced_cfg_insts
    );

    let mut errors = Vec::new();

    // push each unsynced config instance to the server and update the cache
    for unsynced_cfg_inst in unsynced_cfg_insts {
        let inst = unsynced_cfg_inst.value;

        // define the updates
        let activity_status = ActivityStatus::to_backend(&inst.activity_status);
        let error_status = ErrorStatus::to_backend(&inst.error_status);
        let updates = UpdateConfigInstanceRequest {
            activity_status: Some(activity_status),
            error_status: Some(error_status),
        };

        // send to the server
        debug!(
            "Pushing config instance {} to the server with updates: {:?}",
            inst.id, updates
        );
        if let Err(e) = http_client
            .update_config_instance(&inst.id, &updates, token)
            .await
            .map_err(SyncErr::from)
        {
            error!(
                "Failed to push config instance {} to backend: {}",
                inst.id, e
            );
            errors.push(e);
            continue;
        }

        // update the cache
        debug!("Updating cache for config instance {}", inst.id);
        let inst_id = inst.id.clone();
        if let Err(e) = cfg_inst_cache
            .write(inst.id.clone(), inst, |_, _| false, true)
            .await
            .map_err(SyncErr::from)
        {
            error!(
                "Failed to update cache for config instance {} after pushing to the server: {}",
                inst_id, e
            );
            errors.push(e);
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(SyncErr::SyncErrors(SyncErrors {
            errors,
            trace: trace!(),
        }))
    }
}
