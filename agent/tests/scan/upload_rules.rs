// standard crates
use std::collections::BTreeSet;
use std::sync::Arc;

// internal crates
use miru_agent::disk::{self, Layout};
use miru_agent::filesys;
use miru_agent::models::{Deployment, DplActivity, Release, UploadRule, UploadRuleSource};
use miru_agent::scan::upload_rules::{find_deployed, get_dpl_upload_rules};
use miru_agent::scan::ScanErr;

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
    Arc<disk::Deployments>,
    Arc<disk::Releases>,
    Arc<disk::UploadRules>,
) {
    let (deployments, _h1) = disk::Deployments::spawn(64, layout.deployments(), 1000)
        .await
        .unwrap();
    let (releases, _h2) = disk::Releases::spawn(64, layout.releases(), 1000)
        .await
        .unwrap();
    let (upload_rules, _h3) = disk::UploadRules::spawn(64, layout.upload_rules(), 1000)
        .await
        .unwrap();
    (
        Arc::new(deployments),
        Arc::new(releases),
        Arc::new(upload_rules),
    )
}

/// Seed a Deployed deployment -> release -> upload rule bodies so the
/// deployment-scoped traversal resolves to the given rules.
async fn seed_deployed(
    deployments: &disk::Deployments,
    releases: &disk::Releases,
    upload_rules: &disk::UploadRules,
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

// ============================= find_deployed ============================== //

// A Deployed deployment is returned by find_deployed.
#[tokio::test]
async fn find_deployed_returns_deployed() {
    let dir = filesys::dirs::create_temp("testing").await.unwrap();
    let layout = Layout::new(dir);
    let (deployments, releases, upload_rules) = spawn_stores(&layout).await;
    let r1 = rule_with("r1", "/none/*.mcap", 0);
    seed_deployed(
        &deployments,
        &releases,
        &upload_rules,
        "dpl",
        "rel",
        std::slice::from_ref(&r1),
    )
    .await;

    let deployed = find_deployed(deployments.as_ref()).await.unwrap();
    let deployed = deployed.expect("expected a deployed deployment");
    assert_eq!(deployed.id, "dpl");
    assert_eq!(deployed.release_id, "rel");
}

// With only a Queued deployment, find_deployed returns None.
#[tokio::test]
async fn find_deployed_none_when_only_queued() {
    let dir = filesys::dirs::create_temp("testing").await.unwrap();
    let layout = Layout::new(dir);
    let (deployments, releases, upload_rules) = spawn_stores(&layout).await;
    let _ = (&releases, &upload_rules);
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

    assert!(find_deployed(deployments.as_ref()).await.unwrap().is_none());
}

// A shut-down deployments store makes find_deployed return an error (it is NOT
// swallowed to None as the old union-based traversal assumed).
#[tokio::test]
async fn find_deployed_errors_when_store_shut_down() {
    let dir = filesys::dirs::create_temp("testing").await.unwrap();
    let layout = Layout::new(dir);
    let (deployments, _releases, _upload_rules) = spawn_stores(&layout).await;
    deployments.shutdown().await.unwrap();

    let err = find_deployed(deployments.as_ref()).await.unwrap_err();
    assert!(matches!(err, ScanErr::CacheErr(_)));
}

// ========================= get_dpl_upload_rules ========================== //

// Deployed deployment -> release ids [r1,r2] with both bodies present resolves
// to both rules.
#[tokio::test]
async fn resolves_active_set() {
    let dir = filesys::dirs::create_temp("testing").await.unwrap();
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

    let deployed = find_deployed(deployments.as_ref())
        .await
        .unwrap()
        .expect("deployed");
    let rules = get_dpl_upload_rules(releases.as_ref(), upload_rules.as_ref(), deployed)
        .await
        .unwrap();
    assert_eq!(
        ids(&rules),
        BTreeSet::from(["r1".to_string(), "r2".to_string()])
    );
}

// A rule body present in the store but NOT referenced by the deployed release is
// not resolved.
#[tokio::test]
async fn stale_rule_not_acted_on() {
    let dir = filesys::dirs::create_temp("testing").await.unwrap();
    let layout = Layout::new(dir);
    let (deployments, releases, upload_rules) = spawn_stores(&layout).await;
    let r1 = rule_with("r1", "/none/*.mcap", 0);
    // release references only r1, but r2's body also lives in the store.
    seed_deployed(
        &deployments,
        &releases,
        &upload_rules,
        "dpl",
        "rel",
        std::slice::from_ref(&r1),
    )
    .await;
    upload_rules
        .write_if_absent(
            "r2".to_string(),
            rule_with("r2", "/none/*.mcap", 0),
            |_, _| false,
        )
        .await
        .unwrap();

    let deployed = find_deployed(deployments.as_ref())
        .await
        .unwrap()
        .expect("deployed");
    let rules = get_dpl_upload_rules(releases.as_ref(), upload_rules.as_ref(), deployed)
        .await
        .unwrap();
    assert_eq!(ids(&rules), BTreeSet::from(["r1".to_string()]));
}

// The release references [r1,r2] but only r1's body is in the store: the missing
// rule body propagates as an error (NOT silently skipped — the API is a Result).
#[tokio::test]
async fn missing_rule_body_errors() {
    let dir = filesys::dirs::create_temp("testing").await.unwrap();
    let layout = Layout::new(dir);
    let (deployments, releases, upload_rules) = spawn_stores(&layout).await;
    let r1 = rule_with("r1", "/none/*.mcap", 0);
    let r2 = rule_with("r2", "/none/*.mcap", 0);
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

    let deployed = find_deployed(deployments.as_ref())
        .await
        .unwrap()
        .expect("deployed");
    let err = get_dpl_upload_rules(releases.as_ref(), upload_rules.as_ref(), deployed)
        .await
        .unwrap_err();
    // a missing body reads as CacheElementNotFound, wrapped as ScanErr::CacheErr.
    assert!(matches!(err, ScanErr::CacheErr(_)));
}

// A Deployed deployment whose release_id is not cached errors.
#[tokio::test]
async fn missing_release_errors() {
    let dir = filesys::dirs::create_temp("testing").await.unwrap();
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

    let deployed = find_deployed(deployments.as_ref())
        .await
        .unwrap()
        .expect("deployed");
    let err = get_dpl_upload_rules(releases.as_ref(), upload_rules.as_ref(), deployed)
        .await
        .unwrap_err();
    assert!(matches!(err, ScanErr::CacheErr(_)));
}

// A shut-down releases store makes get_dpl_upload_rules error (replacing the old
// "treated as empty" assertion which no longer holds).
#[tokio::test]
async fn release_store_shutdown_errors() {
    let dir = filesys::dirs::create_temp("testing").await.unwrap();
    let layout = Layout::new(dir);
    let (deployments, releases, upload_rules) = spawn_stores(&layout).await;
    let r1 = rule_with("r1", "/none/*.mcap", 0);
    seed_deployed(
        &deployments,
        &releases,
        &upload_rules,
        "dpl",
        "rel",
        std::slice::from_ref(&r1),
    )
    .await;
    let deployed = find_deployed(deployments.as_ref())
        .await
        .unwrap()
        .expect("deployed");
    releases.shutdown().await.unwrap();

    let err = get_dpl_upload_rules(releases.as_ref(), upload_rules.as_ref(), deployed)
        .await
        .unwrap_err();
    assert!(matches!(err, ScanErr::CacheErr(_)));
}

// A shut-down upload_rules store makes get_dpl_upload_rules error.
#[tokio::test]
async fn rule_store_shutdown_errors() {
    let dir = filesys::dirs::create_temp("testing").await.unwrap();
    let layout = Layout::new(dir);
    let (deployments, releases, upload_rules) = spawn_stores(&layout).await;
    let r1 = rule_with("r1", "/none/*.mcap", 0);
    seed_deployed(
        &deployments,
        &releases,
        &upload_rules,
        "dpl",
        "rel",
        std::slice::from_ref(&r1),
    )
    .await;
    let deployed = find_deployed(deployments.as_ref())
        .await
        .unwrap()
        .expect("deployed");
    upload_rules.shutdown().await.unwrap();

    let err = get_dpl_upload_rules(releases.as_ref(), upload_rules.as_ref(), deployed)
        .await
        .unwrap_err();
    assert!(matches!(err, ScanErr::CacheErr(_)));
}
