// standard crates
use std::collections::BTreeSet;

// internal crates
use miru_agent::filesys;
use miru_agent::models::{Deployment, DplActivity, Release, UploadRule, UploadRuleSource};
use miru_agent::storage::{self, Layout};
use miru_agent::sync::upload_rules::active_upload_rules;

// =============================== TEST HELPERS ================================= //

/// Build an UploadRule from Default with only the source fields set.
fn rule_with(id: &str, glob: &str, stability_window_secs: i32) -> UploadRule {
    UploadRule {
        id: id.to_string(),
        source: UploadRuleSource {
            glob: glob.to_string(),
            stability_window_secs,
        },
        ..Default::default()
    }
}

/// The set of resolved rule ids, order-independent.
fn ids(rules: &[UploadRule]) -> BTreeSet<String> {
    rules.iter().map(|r| r.id.clone()).collect()
}

#[allow(clippy::type_complexity)]
async fn spawn_stores(
    layout: &Layout,
) -> (
    std::sync::Arc<storage::Deployments>,
    std::sync::Arc<storage::Releases>,
    std::sync::Arc<storage::UploadRules>,
) {
    use std::sync::Arc;
    let (deployments, _h1) = storage::Deployments::spawn(64, layout.deployments(), 1000)
        .await
        .unwrap();
    let (releases, _h2) = storage::Releases::spawn(64, layout.releases(), 1000)
        .await
        .unwrap();
    let (upload_rules, _h3) = storage::UploadRules::spawn(64, layout.upload_rules(), 1000)
        .await
        .unwrap();
    (
        Arc::new(deployments),
        Arc::new(releases),
        Arc::new(upload_rules),
    )
}

/// Seed a Deployed deployment -> release -> upload rule bodies so the
/// traversal in `active_upload_rules` resolves to the given rules.
async fn seed_deployed(
    deployments: &storage::Deployments,
    releases: &storage::Releases,
    upload_rules: &storage::UploadRules,
    dpl_id: &str,
    release_id: &str,
    rules: &[UploadRule],
) {
    deployments
        .write_if_absent(
            dpl_id.to_string(),
            Deployment {
                id: dpl_id.to_string(),
                activity_status: DplActivity::Deployed,
                release_id: release_id.to_string(),
                ..Default::default()
            },
            |_, _| false,
        )
        .await
        .unwrap();
    releases
        .write_if_absent(
            release_id.to_string(),
            Release {
                id: release_id.to_string(),
                upload_rule_ids: rules.iter().map(|r| r.id.clone()).collect(),
                ..Default::default()
            },
            |_, _| false,
        )
        .await
        .unwrap();
    for rule in rules {
        upload_rules
            .write_if_absent(rule.id.clone(), rule.clone(), |_, _| false)
            .await
            .unwrap();
    }
}

// ====================== active_upload_rules TRAVERSAL ===================== //

// TS3a: Deployed deployment -> release ids [r1,r2] with both bodies
// present => both rules resolve.
#[tokio::test]
async fn resolves_active_set() {
    let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
    let layout = Layout::new(dir);
    let (deployments, releases, upload_rules) = spawn_stores(&layout).await;
    let r1 = rule_with("r1", "/none/*.mcap", 0);
    let r2 = rule_with("r2", "/none/*.mcap", 0);
    seed_deployed(
        &deployments,
        &releases,
        &upload_rules,
        "dpl",
        "rel",
        &[r1, r2],
    )
    .await;

    let rules = active_upload_rules(
        deployments.as_ref(),
        releases.as_ref(),
        upload_rules.as_ref(),
    )
    .await;
    assert_eq!(
        ids(&rules),
        BTreeSet::from(["r1".to_string(), "r2".to_string()])
    );
}

// TS3b: a rule body present in the store but NOT referenced by the
// deployed release is not acted on.
#[tokio::test]
async fn stale_rule_not_acted_on() {
    let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
    let layout = Layout::new(dir);
    let (deployments, releases, upload_rules) = spawn_stores(&layout).await;
    let r1 = rule_with("r1", "/none/*.mcap", 0);
    // release references only r1, but r2's body also lives in the store.
    seed_deployed(&deployments, &releases, &upload_rules, "dpl", "rel", &[r1]).await;
    upload_rules
        .write_if_absent(
            "r2".to_string(),
            rule_with("r2", "/none/*.mcap", 0),
            |_, _| false,
        )
        .await
        .unwrap();

    let rules = active_upload_rules(
        deployments.as_ref(),
        releases.as_ref(),
        upload_rules.as_ref(),
    )
    .await;
    assert_eq!(ids(&rules), BTreeSet::from(["r1".to_string()]));
}

// TS3c: no Deployed deployment (only a Queued one) => empty.
#[tokio::test]
async fn no_deployed_is_empty() {
    let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
    let layout = Layout::new(dir);
    let (deployments, releases, upload_rules) = spawn_stores(&layout).await;
    deployments
        .write_if_absent(
            "dpl".to_string(),
            Deployment {
                id: "dpl".to_string(),
                activity_status: DplActivity::Queued,
                release_id: "rel".to_string(),
                ..Default::default()
            },
            |_, _| false,
        )
        .await
        .unwrap();

    let rules = active_upload_rules(
        deployments.as_ref(),
        releases.as_ref(),
        upload_rules.as_ref(),
    )
    .await;
    assert!(rules.is_empty());
}

// TS3d: release references [r1,r2] but only r1's body is in the store =>
// missing rule id is skipped.
#[tokio::test]
async fn missing_rule_id_skipped() {
    let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
    let layout = Layout::new(dir);
    let (deployments, releases, upload_rules) = spawn_stores(&layout).await;
    let r1 = rule_with("r1", "/none/*.mcap", 0);
    let r2 = rule_with("r2", "/none/*.mcap", 0);
    // seed release referencing both, but only write r1's body.
    deployments
        .write_if_absent(
            "dpl".to_string(),
            Deployment {
                id: "dpl".to_string(),
                activity_status: DplActivity::Deployed,
                release_id: "rel".to_string(),
                ..Default::default()
            },
            |_, _| false,
        )
        .await
        .unwrap();
    releases
        .write_if_absent(
            "rel".to_string(),
            Release {
                id: "rel".to_string(),
                upload_rule_ids: vec![r1.id.clone(), r2.id.clone()],
                ..Default::default()
            },
            |_, _| false,
        )
        .await
        .unwrap();
    upload_rules
        .write_if_absent(r1.id.clone(), r1, |_, _| false)
        .await
        .unwrap();

    let rules = active_upload_rules(
        deployments.as_ref(),
        releases.as_ref(),
        upload_rules.as_ref(),
    )
    .await;
    assert_eq!(ids(&rules), BTreeSet::from(["r1".to_string()]));
}

// TS3d2: Deployed deployment whose release_id is not cached => empty.
#[tokio::test]
async fn missing_release_is_empty() {
    let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
    let layout = Layout::new(dir);
    let (deployments, releases, upload_rules) = spawn_stores(&layout).await;
    deployments
        .write_if_absent(
            "dpl".to_string(),
            Deployment {
                id: "dpl".to_string(),
                activity_status: DplActivity::Deployed,
                release_id: "absent".to_string(),
                ..Default::default()
            },
            |_, _| false,
        )
        .await
        .unwrap();

    let rules = active_upload_rules(
        deployments.as_ref(),
        releases.as_ref(),
        upload_rules.as_ref(),
    )
    .await;
    assert!(rules.is_empty());
}

// TS3e: two Deployed deployments referencing overlapping rule ids union
// and dedupe to a single set.
#[tokio::test]
async fn union_and_dedupe_across_deployments() {
    let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
    let layout = Layout::new(dir);
    let (deployments, releases, upload_rules) = spawn_stores(&layout).await;
    let r1 = rule_with("r1", "/none/*.mcap", 0);
    let r2 = rule_with("r2", "/none/*.mcap", 0);
    seed_deployed(
        &deployments,
        &releases,
        &upload_rules,
        "dpl_A",
        "rel_A",
        std::slice::from_ref(&r1),
    )
    .await;
    seed_deployed(
        &deployments,
        &releases,
        &upload_rules,
        "dpl_B",
        "rel_B",
        &[r1, r2],
    )
    .await;

    let rules = active_upload_rules(
        deployments.as_ref(),
        releases.as_ref(),
        upload_rules.as_ref(),
    )
    .await;
    assert_eq!(
        ids(&rules),
        BTreeSet::from(["r1".to_string(), "r2".to_string()])
    );
    assert_eq!(rules.len(), 2);
}

// TS3-err-deployments: deployments store shut down => find_where errors,
// treated as empty.
#[tokio::test]
async fn deployments_cache_error_is_empty() {
    let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
    let layout = Layout::new(dir);
    let (deployments, releases, upload_rules) = spawn_stores(&layout).await;
    deployments.shutdown().await.unwrap();

    let rules = active_upload_rules(
        deployments.as_ref(),
        releases.as_ref(),
        upload_rules.as_ref(),
    )
    .await;
    assert!(rules.is_empty());
}

// TS3-err-release: releases store shut down => read_optional errors for
// the deployed release, that deployment is skipped.
#[tokio::test]
async fn release_cache_error_is_empty() {
    let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
    let layout = Layout::new(dir);
    let (deployments, releases, upload_rules) = spawn_stores(&layout).await;
    let r1 = rule_with("r1", "/none/*.mcap", 0);
    seed_deployed(&deployments, &releases, &upload_rules, "dpl", "rel", &[r1]).await;
    releases.shutdown().await.unwrap();

    let rules = active_upload_rules(
        deployments.as_ref(),
        releases.as_ref(),
        upload_rules.as_ref(),
    )
    .await;
    assert!(rules.is_empty());
}

// TS3-err-rule: upload_rules store shut down => read_optional errors per
// rule id, all skipped.
#[tokio::test]
async fn rule_cache_error_is_empty() {
    let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
    let layout = Layout::new(dir);
    let (deployments, releases, upload_rules) = spawn_stores(&layout).await;
    let r1 = rule_with("r1", "/none/*.mcap", 0);
    seed_deployed(&deployments, &releases, &upload_rules, "dpl", "rel", &[r1]).await;
    upload_rules.shutdown().await.unwrap();

    let rules = active_upload_rules(
        deployments.as_ref(),
        releases.as_ref(),
        upload_rules.as_ref(),
    )
    .await;
    assert!(rules.is_empty());
}
