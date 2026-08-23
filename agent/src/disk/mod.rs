// standard crates
use std::future::Future;
use std::sync::Arc;

pub mod agent_version;
pub mod config_instances;
pub mod deployments;
pub mod device;
pub mod errors;
pub mod file_rules;
pub mod git_commits;
pub mod layout;
pub mod releases;
pub mod settings;
pub mod setup;

pub use self::config_instances::{CfgInstContent, CfgInsts};
pub use self::deployments::{Deployments, DplEntry};
pub use self::device::{activation_state, assert_activated, resolve_device_id, Activation, Device};
pub use self::errors::{DeviceNotActivatedErr, DiskErr};
pub use self::file_rules::{file_rules_for_deployed, file_rules_for_deployment, FileRules};
pub use self::git_commits::GitCommits;
pub use self::layout::Layout;
pub use self::releases::Releases;
pub use self::settings::{Backend, MQTTBroker, Settings};
pub use crate::network::{BackendHost, MqttHost};

use self::device::Device as DeviceStorage;
use self::errors::DiskErr as StorErr;
use self::layout::Layout as StorLayout;
use crate::filesys::Overwrite;
use crate::models;

use tokio::task::JoinHandle;
use tracing::{error, info};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Capacities {
    pub cfg_insts: usize,
    pub cfg_inst_content: usize,
    pub deployments: usize,
    pub releases: usize,
    pub file_rules: usize,
    pub git_commits: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            cfg_insts: 1000,
            cfg_inst_content: 1000,
            deployments: 100,
            releases: 1000,
            file_rules: 1000,
            git_commits: 100,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CfgInstStor {
    pub meta: Arc<CfgInsts>,
    pub content: Arc<CfgInstContent>,
}

pub struct CfgInstRef<'a> {
    pub meta: &'a CfgInsts,
    pub content: &'a CfgInstContent,
}

impl CfgInstStor {
    pub fn as_ref(&self) -> CfgInstRef<'_> {
        CfgInstRef {
            meta: &self.meta,
            content: &self.content,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Storage {
    pub device: Arc<DeviceStorage>,
    pub cfg_insts: CfgInstStor,
    pub deployments: Arc<Deployments>,
    pub releases: Arc<Releases>,
    pub file_rules: Arc<FileRules>,
    pub git_commits: Arc<GitCommits>,
}

impl Storage {
    pub async fn init(
        layout: &StorLayout,
        capacities: Capacities,
        device_id: String,
    ) -> Result<(Storage, impl Future<Output = ()>), StorErr> {
        // device storage
        let (device, device_handle) = init_device_storage(layout, device_id).await?;

        // config instances (metadata + content)
        let (cfg_insts, cfg_inst_handles) = init_cfg_inst_storage(layout, capacities).await?;

        // deployments
        let (deployments, deployment_handle) = init_deployment_storage(layout, capacities).await?;

        // releases
        let (release_stor, release_handle) =
            Releases::spawn(64, layout.releases(), capacities.releases).await?;
        let releases = Arc::new(release_stor);

        // file rules
        let (file_rule_stor, file_rule_handle) =
            FileRules::spawn(64, layout.file_rules(), capacities.file_rules).await?;
        let file_rules = Arc::new(file_rule_stor);

        // git commits
        let (git_commit_stor, git_commit_handle) =
            GitCommits::spawn(64, layout.git_commits(), capacities.git_commits).await?;
        let git_commits = Arc::new(git_commit_stor);

        let shutdown_handle = async move {
            let mut handles = vec![device_handle];
            handles.extend(cfg_inst_handles);
            handles.extend([
                deployment_handle,
                release_handle,
                file_rule_handle,
                git_commit_handle,
            ]);

            futures::future::join_all(handles).await;
        };

        Ok((
            Storage {
                device,
                cfg_insts,
                deployments,
                releases,
                file_rules,
                git_commits,
            },
            shutdown_handle,
        ))
    }

    // if the device is online, set it to offline before shutting down
    async fn mark_device_offline(&self, first_err: Option<StorErr>) -> Option<StorErr> {
        let device_data = match self.device.read().await {
            Ok(device_data) => device_data,
            Err(e) => {
                error!("failed to read device data during shutdown: {e}");
                return first_err.or(Some(e.into()));
            }
        };

        match device_data.status {
            models::DeviceStatus::Online => {
                info!("Shutting down device storage, setting device to offline");
                record(
                    first_err,
                    "device offline patch",
                    self.device
                        .patch(models::device::Updates::disconnected())
                        .await,
                )
            }
            models::DeviceStatus::Offline => {
                info!("Shutting down device storage, device is already offline");
                first_err
            }
        }
    }

    pub async fn shutdown(&self) -> Result<(), StorErr> {
        // best-effort: attempt every step, return the first error at the end
        let mut first_err: Option<StorErr> = None;

        first_err = self.mark_device_offline(first_err).await;

        first_err = record(first_err, "device store", self.device.shutdown().await);
        first_err = record(
            first_err,
            "config instance metadata store",
            self.cfg_insts.meta.shutdown().await,
        );
        first_err = record(
            first_err,
            "config instance content store",
            self.cfg_insts.content.shutdown().await,
        );
        first_err = record(
            first_err,
            "deployments store",
            self.deployments.shutdown().await,
        );
        first_err = record(first_err, "releases store", self.releases.shutdown().await);
        first_err = record(
            first_err,
            "file rules store",
            self.file_rules.shutdown().await,
        );
        first_err = record(
            first_err,
            "git commits store",
            self.git_commits.shutdown().await,
        );

        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

// Logs a failed shutdown step and folds it into the running first error:
// the earliest error wins, later ones are logged only.
fn record<E: Into<StorErr>>(
    first_err: Option<StorErr>,
    target: &str,
    result: Result<(), E>,
) -> Option<StorErr> {
    match result {
        Ok(()) => first_err,
        Err(e) => {
            let e = e.into();
            error!("failed to shutdown {target}: {e}");
            first_err.or(Some(e))
        }
    }
}

async fn init_device_storage(
    layout: &StorLayout,
    device_id: String,
) -> Result<(Arc<DeviceStorage>, JoinHandle<()>), StorErr> {
    let (device_storage, device_storage_handle) = DeviceStorage::spawn_with_default(
        64,
        layout.device(),
        models::Device {
            id: device_id.clone(),
            activated: true,
            status: models::DeviceStatus::Offline,
            ..models::Device::default()
        },
    )
    .await?;

    device_storage
        .patch(models::device::Updates {
            status: Some(models::DeviceStatus::Offline),
            ..models::device::Updates::empty()
        })
        .await?;

    Ok((Arc::new(device_storage), device_storage_handle))
}

async fn init_cfg_inst_storage(
    layout: &StorLayout,
    capacities: Capacities,
) -> Result<(CfgInstStor, [JoinHandle<()>; 2]), StorErr> {
    // config instance metadata
    let (cfg_inst_stor, cfg_inst_stor_handle) =
        CfgInsts::spawn(64, layout.config_instance_meta(), capacities.cfg_insts).await?;

    // config instance content
    let (cfg_inst_content_stor, cfg_inst_content_stor_handle) = CfgInstContent::spawn(
        64,
        layout.config_instance_content(),
        capacities.cfg_inst_content,
    )
    .await?;

    let cfg_insts = CfgInstStor {
        meta: Arc::new(cfg_inst_stor),
        content: Arc::new(cfg_inst_content_stor),
    };
    Ok((
        cfg_insts,
        [cfg_inst_stor_handle, cfg_inst_content_stor_handle],
    ))
}

async fn init_deployment_storage(
    layout: &StorLayout,
    capacities: Capacities,
) -> Result<(Arc<Deployments>, JoinHandle<()>), StorErr> {
    let (deployment_stor, deployment_stor_handle) =
        Deployments::spawn(64, layout.deployments(), capacities.deployments).await?;
    reset_deployment_retry_state(&deployment_stor).await?;
    Ok((Arc::new(deployment_stor), deployment_stor_handle))
}

/// Resets retry state (attempts, cooldown) for all persisted deployments so
/// they are retried immediately after an agent restart. The most common
/// reason for a restart is "I fixed the problem, retry now."
async fn reset_deployment_retry_state(deployments: &Deployments) -> Result<(), StorErr> {
    let entries = deployments
        .find_entries_where(|e| !e.value.has_clean_retry_state())
        .await?;
    for entry in entries {
        let id = entry.key.clone();
        let mut dpl = entry.value;
        dpl.reset_retry_state();
        deployments
            .write(
                id,
                dpl,
                |old, _| old.is_some_and(|e| e.is_dirty),
                Overwrite::Allow,
            )
            .await?;
    }
    Ok(())
}
