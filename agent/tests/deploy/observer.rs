// internal crates
use miru_agent::deploy::errors::DeployErr;
use miru_agent::deploy::observer::Observer;
use miru_agent::models::deployment::Deployment;

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
