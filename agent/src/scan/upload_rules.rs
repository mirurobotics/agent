// standard crates
use std::collections::HashSet;

// internal crates
use crate::disk;
use crate::models::{DplActivity, UploadRule, UploadRuleID};
use crate::scan::collection::Config;

// external crates
use tracing::{debug, error};

/// Resolves the active upload rules from the currently-deployed deployment(s):
/// Deployed deployment -> release (by release_id) -> release.upload_rule_ids
/// -> rule BODIES (by id) from the append-only UploadRules store. Unions across
/// all Deployed deployments (normally exactly one; the union covers redeploy
/// transitions). Missing ids are skipped with a debug log. Cache errors are
/// logged and treated as empty so the worker never crashes.
///
/// Public as a test seam (mirrors `find_stable`) so unit tests can exercise the
/// traversal directly against seeded stores.
pub async fn active_upload_rules(
    deployments: &disk::Deployments,
    releases: &disk::Releases,
    upload_rules: &disk::UploadRules,
) -> Vec<UploadRule> {
    active_upload_configs(deployments, releases, upload_rules)
        .await
        .into_iter()
        .map(|cfg| cfg.rule)
        .collect()
}

/// Like [`active_upload_rules`], but pairs each rule with the deployed
/// deployment that sourced it (needed by the scanner's per-collection state).
pub async fn active_upload_configs(
    deployments: &disk::Deployments,
    releases: &disk::Releases,
    upload_rules: &disk::UploadRules,
) -> Vec<Config> {
    let deployed = match deployments
        .find_where(|d| d.activity_status == DplActivity::Deployed)
        .await
    {
        Ok(d) => d,
        Err(e) => {
            error!("error reading deployments: {e:?}");
            return Vec::new();
        }
    };

    let mut seen: HashSet<UploadRuleID> = HashSet::new();
    let mut out: Vec<Config> = Vec::new();
    for dpl in deployed {
        let release = match releases.read_optional(dpl.release_id.clone()).await {
            Ok(Some(r)) => r,
            Ok(None) => {
                debug!(
                    "release {} for deployed deployment {} not cached; skipping",
                    dpl.release_id, dpl.id
                );
                continue;
            }
            Err(e) => {
                error!("error reading release {}: {e:?}", dpl.release_id);
                continue;
            }
        };
        for rule_id in &release.upload_rule_ids {
            if !seen.insert(rule_id.clone()) {
                continue;
            }
            match upload_rules.read_optional(rule_id.clone()).await {
                Ok(Some(rule)) => out.push(Config {
                    deployment: dpl.clone(),
                    rule,
                }),
                Ok(None) => debug!(
                    "upload rule {rule_id} referenced by release {} not in store; skipping",
                    release.id
                ),
                Err(e) => error!("error reading upload rule {rule_id}: {e:?}"),
            }
        }
    }
    out
}
