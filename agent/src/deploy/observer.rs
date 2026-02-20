// internal crates
use crate::deploy::errors::DeployErr;
use crate::filesys::Overwrite;
use crate::models::deployment::Deployment;
use crate::storage;

// external crates
use async_trait::async_trait;

#[async_trait]
pub trait Observer: Send {
    async fn on_update(&mut self, deployment: &Deployment) -> Result<(), DeployErr>;
}

pub async fn on_update(
    observers: &mut [&mut dyn Observer],
    deployment: &Deployment,
) -> Result<(), DeployErr> {
    for observer in observers.iter_mut() {
        observer.on_update(deployment).await?
    }
    Ok(())
}

pub struct Storage<'a> {
    pub deployment_stor: &'a storage::Deployments,
}

#[async_trait]
impl<'a> Observer for Storage<'a> {
    async fn on_update(&mut self, deployment: &Deployment) -> Result<(), DeployErr> {
        self.deployment_stor
            .write(
                deployment.id.clone(),
                deployment.clone(),
                storage::deployments::is_dirty,
                Overwrite::Allow,
            )
            .await
            .map_err(DeployErr::from)
    }
}
