// internal crates
use crate::deploy::errors::DeployErr;
use crate::models::deployment::Deployment;

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
