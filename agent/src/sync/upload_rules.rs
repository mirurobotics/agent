// standard crates
use std::collections::HashSet;

// internal crates
use crate::models;
use crate::storage;
use crate::sync::errors::SyncErr;

// external crates
use tracing::warn;

pub struct Storage<'a> {
    pub deployments: &'a storage::Deployments,
    pub releases: &'a storage::Releases,
    pub upload_rules: &'a storage::UploadRules,
}

/// Resolves the active upload-rule set by traversal: currently-deployed
/// deployment(s) → their release → the release's `upload_rule_ids` → the rule
/// bodies in the append-only upload-rules store. Rules cached from a
/// since-replaced release are never pruned from the store; they simply stop
/// being reachable from a deployed deployment and are ignored here.
///
/// The result is the union across all deployed deployments (normally a
/// singleton — the union avoids dropping an outgoing release's rules
/// mid-redeploy), deduped by rule id. No deployed deployment yields an empty
/// set. Missing releases or rule bodies are skipped with a warning so a
/// partially-cached deployment cannot fail the sync; storage errors propagate.
pub async fn active_upload_rules(
    storage: &Storage<'_>,
) -> Result<Vec<models::UploadRule>, SyncErr> {
    let deployed = storage
        .deployments
        .find_where(|dpl| dpl.activity_status == models::DplActivity::Deployed)
        .await?;

    let mut seen: HashSet<models::UploadRuleID> = HashSet::new();
    let mut rules = Vec::new();
    for dpl in deployed {
        let release = storage
            .releases
            .read_optional(dpl.release_id.clone())
            .await?;
        let Some(release) = release else {
            warn!(
                "release '{}' for deployed deployment '{}' is not cached; skipping its upload rules",
                dpl.release_id, dpl.id
            );
            continue;
        };
        for rule_id in release.upload_rule_ids {
            if !seen.insert(rule_id.clone()) {
                continue;
            }
            let rule = storage.upload_rules.read_optional(rule_id.clone()).await?;
            let Some(rule) = rule else {
                warn!(
                    "upload rule '{}' referenced by release '{}' is not cached; skipping it",
                    rule_id, release.id
                );
                continue;
            };
            rules.push(rule);
        }
    }
    Ok(rules)
}
