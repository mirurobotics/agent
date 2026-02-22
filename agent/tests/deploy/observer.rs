// internal crates
use miru_agent::deploy::errors::DeployErr;
use miru_agent::deploy::observer::Observer;
use miru_agent::models::deployment::Deployment;
use miru_agent::storage::StorageErr;

// external crates
use async_trait::async_trait;

#[derive(Debug, Default)]
pub struct HistoryObserver {
    pub history: Vec<Deployment>,
}

impl HistoryObserver {
    pub fn new() -> Self {
        Self {
            history: Vec::new(),
        }
    }
}

#[async_trait]
impl Observer for HistoryObserver {
    async fn on_update(&mut self, deployment: &Deployment) -> Result<(), DeployErr> {
        self.history.push(deployment.clone());
        Ok(())
    }
}

/// Test-only observer that always returns an error.
///
/// Uses `DeployErr::StorageErr` because the production `Storage` observer
/// writes to the deployment store, so a storage error is the most realistic
/// failure mode. The inner `CacheElementNotFound` is the simplest
/// constructable leaf — treat it as an opaque test stub.
pub struct FailingObserver;

#[async_trait]
impl Observer for FailingObserver {
    async fn on_update(&mut self, _deployment: &Deployment) -> Result<(), DeployErr> {
        Err(DeployErr::StorageErr(StorageErr::CacheErr(
            miru_agent::cache::errors::CacheErr::CacheElementNotFound(
                miru_agent::cache::errors::CacheElementNotFound {
                    msg: "test stub: simulated observer failure".to_string(),
                    trace: miru_agent::trace!(),
                },
            ),
        )))
    }
}
