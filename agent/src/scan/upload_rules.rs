// internal crates
use crate::disk;
use crate::models::{DplActivity, Deployment, Release, UploadRule};
use crate::scan::errors::*;

pub async fn find_deployed(
    deployments: &disk::Deployments,
) -> Result<Option<Deployment>, ScanErr> {
    deployments.find_one_optional(
        "deployed",
        |d| d.activity_status == DplActivity::Deployed,
    ).await.map_err(ScanErr::from)
}

pub async fn get_dpl_upload_rules(
    releases: &disk::Releases,
    upload_rules: &disk::UploadRules,
    deployment: Deployment,
) -> Result<Vec<UploadRule>, ScanErr> {
    let release = get_dpl_rls(releases, deployment).await?;
    get_rls_upload_rules(upload_rules, release).await
}

async fn get_dpl_rls(
    releases: &disk::Releases,
    deployment: Deployment,
) -> Result<Release, ScanErr> {
    releases.read(deployment.release_id.clone()).await.map_err(ScanErr::from)
}

async fn get_rls_upload_rules(
    upload_rules: &disk::UploadRules,
    release: Release,
) -> Result<Vec<UploadRule>, ScanErr> {
    let mut rules: Vec<UploadRule> = Vec::new();
    for rule_id in &release.upload_rule_ids {
        let rule = upload_rules.read(rule_id.clone()).await?;
        rules.push(rule);
    }
    Ok(rules)
}
