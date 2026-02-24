use chrono::{DateTime, TimeDelta, Utc};
use miru_agent::models;
use miru_agent::storage::{CfgInstContent, CfgInsts, Deployments};
use miru_agent::sync::syncer::State;
use openapi_client::models::{
    Deployment as BackendDeployment, DeploymentActivityStatus as BackendActivityStatus,
    DeploymentTargetStatus as BackendTargetStatus,
};

// ========================= FACTORIES ========================= //

pub fn make_cfg_inst(id: &str) -> openapi_client::models::ConfigInstance {
    openapi_client::models::ConfigInstance {
        id: id.to_string(),
        filepath: format!("{id}.json"),
        ..Default::default()
    }
}

pub fn make_deployment(id: &str, cfg_inst_ids: &[&str]) -> BackendDeployment {
    let cfg_insts: Vec<_> = cfg_inst_ids.iter().map(|id| make_cfg_inst(id)).collect();
    BackendDeployment {
        id: id.to_string(),
        activity_status: BackendActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_QUEUED,
        target_status: BackendTargetStatus::DEPLOYMENT_TARGET_STATUS_DEPLOYED,
        config_instances: Some(cfg_insts),
        ..Default::default()
    }
}

pub fn make_archived_dpl(id: &str, cfg_inst_ids: &[&str]) -> BackendDeployment {
    BackendDeployment {
        target_status: BackendTargetStatus::DEPLOYMENT_TARGET_STATUS_ARCHIVED,
        ..make_deployment(id, cfg_inst_ids)
    }
}

// ========================= ASSERTIONS ========================= //

pub async fn read_deployment(deployment_stor: &Deployments, id: &str) -> models::Deployment {
    let cached = deployment_stor
        .read_optional(id.to_string())
        .await
        .unwrap()
        .expect("deployment should be stored");
    cached
}

pub async fn assert_deployment_stored(deployment_stor: &Deployments, id: &str) {
    read_deployment(deployment_stor, id).await;
}

pub async fn assert_deployment_not_stored(deployment_stor: &Deployments, id: &str) {
    assert!(
        deployment_stor
            .read_optional(id.to_string())
            .await
            .unwrap()
            .is_none(),
        "deployment should not be stored"
    );
}

pub async fn read_cfg_inst(cfg_inst_stor: &CfgInsts, id: &str) -> models::ConfigInstance {
    let cached = cfg_inst_stor
        .read_optional(id.to_string())
        .await
        .unwrap()
        .expect("config instance should be stored");
    cached
}

pub async fn read_content(cfg_inst_content_stor: &CfgInstContent, id: &str) -> String {
    let cached = cfg_inst_content_stor
        .read_optional(id.to_string())
        .await
        .unwrap()
        .expect("config instance content should be stored");
    cached
}

pub async fn assert_cfg_inst_stored(cfg_inst_stor: &CfgInsts, id: &str) {
    read_cfg_inst(cfg_inst_stor, id).await;
}

pub async fn assert_content_not_stored(cfg_inst_content_stor: &CfgInstContent, id: &str) {
    assert!(
        cfg_inst_content_stor
            .read_optional(id.to_string())
            .await
            .unwrap()
            .is_none(),
        "config instance content should not be stored"
    );
}

// ========================= STATE ASSERTIONS ========================= //

/// Captures a before/after time window for asserting syncer `State` timestamps.
pub struct StateAssert {
    pub before: DateTime<Utc>,
    pub after: DateTime<Utc>,
}

impl StateAssert {
    pub fn new(before: DateTime<Utc>, after: DateTime<Utc>) -> Self {
        Self { before, after }
    }

    /// Assert `ts` falls within `[before + offset, after + offset]`.
    pub fn assert_between(&self, ts: DateTime<Utc>, offset: TimeDelta) {
        let lo = self.before + offset;
        let hi = self.after + offset;
        assert!(
            ts >= lo && ts <= hi,
            "expected timestamp in [{lo}, {hi}], got: {ts}"
        );
    }

    /// Assert state after a successful sync:
    /// - `last_attempted_sync_at` and `last_synced_at` within the window
    /// - `cooldown_ends_at` within the window shifted by `cooldown`
    /// - `err_streak` equals the given value
    pub fn assert_success(&self, state: &State, cooldown: TimeDelta, err_streak: u32) {
        self.assert_between(state.last_attempted_sync_at, TimeDelta::zero());
        self.assert_between(state.last_synced_at, TimeDelta::zero());
        self.assert_between(state.cooldown_ends_at, cooldown);
        assert_eq!(state.err_streak, err_streak);
    }

    /// Assert state after a failed sync (last_synced_at unchanged at UNIX_EPOCH):
    /// - `last_attempted_sync_at` within the window
    /// - `last_synced_at` == UNIX_EPOCH
    /// - `cooldown_ends_at` within the window shifted by `cooldown`
    /// - `err_streak` equals the given value
    pub fn assert_failed(&self, state: &State, cooldown: TimeDelta, err_streak: u32) {
        self.assert_between(state.last_attempted_sync_at, TimeDelta::zero());
        assert_eq!(state.last_synced_at, DateTime::<Utc>::UNIX_EPOCH);
        self.assert_between(state.cooldown_ends_at, cooldown);
        assert_eq!(state.err_streak, err_streak);
    }
}
