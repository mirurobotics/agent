// standard crates
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::mocks::syncer::MockSyncer;
use miru_agent::disk::{self, Layout};
use miru_agent::filesys::dirs;
use miru_agent::models::{Deployment, DplActivity, Release, UploadRule, UploadRuleSource};
use miru_agent::scan::scanner::{Scanner, ScannerArgs};
use miru_agent::sync::syncer::SyncEvent;
use miru_agent::workers::scan_bridge;

// external crates
use chrono::Utc;

// =============================== TEST HELPERS ================================= //

fn rule_with(id: &str, glob: &str) -> UploadRule {
    UploadRule {
        id: id.to_string(),
        source: UploadRuleSource {
            glob: glob.to_string(),
            stability_window_secs: 0,
        },
        ..Default::default()
    }
}

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
/// `active_upload_rules` traversal resolves to the given rules.
async fn seed_deployed(
    deployments: &disk::Deployments,
    releases: &disk::Releases,
    upload_rules: &disk::UploadRules,
    rules: &[UploadRule],
) {
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

fn spawn_test_scanner() -> Arc<Scanner> {
    let (scanner, _h) = Scanner::spawn(
        64,
        ScannerArgs {
            min_poll_interval_secs: 1,
            now_fn: Arc::new(Utc::now),
        },
    )
    .unwrap();
    Arc::new(scanner)
}

/// Poll `scanner.get_rules()` until it matches `expected` (by id set) or the
/// deadline elapses, returning the final observed id set. The bridge pushes
/// asynchronously so we cannot read the scanner synchronously right after an
/// event.
async fn await_rules(scanner: &Scanner, expected: &BTreeSet<String>) -> BTreeSet<String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let got = ids(&scanner.get_rules().await.unwrap());
        if &got == expected || std::time::Instant::now() >= deadline {
            return got;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn spawn_bridge(
    scanner: Arc<Scanner>,
    deployments: Arc<disk::Deployments>,
    releases: Arc<disk::Releases>,
    upload_rules: Arc<disk::UploadRules>,
    syncer: Arc<MockSyncer>,
) -> tokio::task::JoinHandle<()> {
    let shutdown_signal = Box::pin(async move {
        std::future::pending::<()>().await;
    });
    tokio::spawn(async move {
        scan_bridge::run(
            scanner.as_ref(),
            deployments.as_ref(),
            releases.as_ref(),
            upload_rules.as_ref(),
            syncer.as_ref(),
            shutdown_signal,
        )
        .await;
    })
}

// ================================== TESTS ==================================== //

pub mod run {
    use super::*;

    // Startup seed: the watch channel's initial value is SyncSuccess, but the
    // bridge must NOT rely on `changed()` to observe it. With a Deployed
    // deployment seeded before the bridge starts, the scanner must receive the
    // resolved rule set at startup without any event being sent.
    #[tokio::test]
    async fn seeds_scanner_at_startup() {
        let dir = dirs::create_temp("scan_bridge_seed").await.unwrap();
        let layout = Layout::new(dir);
        let (deployments, releases, upload_rules) = spawn_stores(&layout).await;
        seed_deployed(
            &deployments,
            &releases,
            &upload_rules,
            &[rule_with("r1", "/none/*.mcap")],
        )
        .await;

        let scanner = spawn_test_scanner();
        let syncer = Arc::new(MockSyncer::default());

        let _handle = spawn_bridge(
            scanner.clone(),
            deployments.clone(),
            releases.clone(),
            upload_rules.clone(),
            syncer.clone(),
        )
        .await;

        // No event is ever sent: the scanner is fed purely by the startup seed.
        let expected = BTreeSet::from(["r1".to_string()]);
        assert_eq!(await_rules(&scanner, &expected).await, expected);
    }

    // After a SyncSuccess event, the bridge recomputes the active set and pushes
    // it into the scanner. The stores are updated AFTER the startup seed (which
    // resolves to empty) so the change is attributable to the event.
    #[tokio::test]
    async fn sync_success_pushes_resolved_active_set() {
        let dir = dirs::create_temp("scan_bridge_push").await.unwrap();
        let layout = Layout::new(dir);
        let (deployments, releases, upload_rules) = spawn_stores(&layout).await;

        let scanner = spawn_test_scanner();
        let syncer = Arc::new(MockSyncer::default());

        let _handle = spawn_bridge(
            scanner.clone(),
            deployments.clone(),
            releases.clone(),
            upload_rules.clone(),
            syncer.clone(),
        )
        .await;

        // Startup seed resolves to empty (no deployment yet).
        assert_eq!(
            await_rules(&scanner, &BTreeSet::new()).await,
            BTreeSet::new()
        );

        // Now a deployment becomes Deployed; the next SyncSuccess must push it.
        seed_deployed(
            &deployments,
            &releases,
            &upload_rules,
            &[rule_with("r1", "/none/*.mcap")],
        )
        .await;
        syncer
            .get_transmitter()
            .send(SyncEvent::SyncSuccess)
            .unwrap();

        let expected = BTreeSet::from(["r1".to_string()]);
        assert_eq!(await_rules(&scanner, &expected).await, expected);
    }

    // With only a Queued deployment, the active set is empty so the scanner ends
    // up with no rules after a SyncSuccess.
    #[tokio::test]
    async fn empty_when_no_deployed() {
        let dir = dirs::create_temp("scan_bridge_empty").await.unwrap();
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

        let scanner = spawn_test_scanner();
        let syncer = Arc::new(MockSyncer::default());

        let _handle = spawn_bridge(
            scanner.clone(),
            deployments.clone(),
            releases.clone(),
            upload_rules.clone(),
            syncer.clone(),
        )
        .await;

        syncer
            .get_transmitter()
            .send(SyncEvent::SyncSuccess)
            .unwrap();

        assert_eq!(
            await_rules(&scanner, &BTreeSet::new()).await,
            BTreeSet::new()
        );
    }

    // Non-SyncSuccess events are ignored: they must not push a (possibly empty)
    // rule set that would clobber the startup seed.
    #[tokio::test]
    async fn ignores_non_sync_success_events() {
        let dir = dirs::create_temp("scan_bridge_ignore").await.unwrap();
        let layout = Layout::new(dir);
        let (deployments, releases, upload_rules) = spawn_stores(&layout).await;
        seed_deployed(
            &deployments,
            &releases,
            &upload_rules,
            &[rule_with("r1", "/none/*.mcap")],
        )
        .await;

        let scanner = spawn_test_scanner();
        let syncer = Arc::new(MockSyncer::default());

        let _handle = spawn_bridge(
            scanner.clone(),
            deployments.clone(),
            releases.clone(),
            upload_rules.clone(),
            syncer.clone(),
        )
        .await;

        // Startup seed pushes r1.
        let expected = BTreeSet::from(["r1".to_string()]);
        assert_eq!(await_rules(&scanner, &expected).await, expected);

        // Drop every rule from the stores, then send an ignored event. Since the
        // bridge ignores it, the scanner must still hold the seeded r1.
        upload_rules.delete("r1".to_string()).await.unwrap();
        releases.delete("rel".to_string()).await.unwrap();
        deployments.delete("dpl".to_string()).await.unwrap();
        syncer
            .get_transmitter()
            .send(SyncEvent::SyncFailed(
                miru_agent::sync::syncer::SyncFailure {
                    is_network_conn_err: false,
                },
            ))
            .unwrap();

        // Give the bridge a moment to (not) react, then confirm r1 persists.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(ids(&scanner.get_rules().await.unwrap()), expected);
    }
}
