use backend_api::models::{
    Deployment as BackendDeployment, DeploymentActivityStatus as BackendActivityStatus,
    DeploymentTargetStatus as BackendTargetStatus, GitCommit as BackendGitCommit,
    GitRepositoryType, Release as BackendRelease,
};
use chrono::{DateTime, TimeDelta, Utc};
use miru_agent::models;
use miru_agent::storage::{CfgInstContent, CfgInsts, Deployments, GitCommits, Releases};
use miru_agent::sync::syncer::State;

// ========================= FACTORIES ========================= //

pub fn make_cfg_inst(id: &str) -> backend_api::models::ConfigInstance {
    backend_api::models::ConfigInstance {
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

pub fn make_backend_git_commit(id: &str) -> BackendGitCommit {
    BackendGitCommit {
        object: backend_api::models::git_commit::Object::GitCommit,
        id: id.to_string(),
        sha: format!("sha-{id}"),
        message: format!("commit {id}"),
        repository_owner: "miru-hq".to_string(),
        repository_name: "miru".to_string(),
        repository_type: GitRepositoryType::GIT_REPO_TYPE_GITHUB,
        repository_url: "https://github.com/miru-hq/miru".to_string(),
        commit_url: format!("https://github.com/miru-hq/miru/commit/sha-{id}"),
        created_at: Utc::now().to_rfc3339(),
    }
}

pub fn make_backend_release(id: &str, gc_id: Option<&str>) -> BackendRelease {
    let git_commit = gc_id.map(|gid| Some(Box::new(make_backend_git_commit(gid))));
    BackendRelease {
        object: backend_api::models::release::Object::Release,
        id: id.to_string(),
        version: format!("1.0.0-{id}"),
        git_commit_id: gc_id.map(|s| s.to_string()),
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
        git_commit,
    }
}

pub fn make_deployment_with_release(
    id: &str,
    cfg_inst_ids: &[&str],
    release_id: &str,
    gc_id: Option<&str>,
) -> BackendDeployment {
    let mut dpl = make_deployment(id, cfg_inst_ids);
    dpl.release_id = release_id.to_string();
    dpl.release = Some(Box::new(make_backend_release(release_id, gc_id)));
    dpl
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

pub async fn assert_release_stored(release_stor: &Releases, id: &str) {
    let cached = release_stor.read_optional(id.to_string()).await.unwrap();
    assert!(cached.is_some(), "release {id} should be stored");
}

pub async fn assert_git_commit_stored(git_commit_stor: &GitCommits, id: &str) {
    let cached = git_commit_stor.read_optional(id.to_string()).await.unwrap();
    assert!(cached.is_some(), "git commit {id} should be stored");
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
