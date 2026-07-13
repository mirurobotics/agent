// standard crates
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::mocks::{scanner::MockScanner, syncer::MockSyncer};
use miru_agent::disk::{self, Layout};
use miru_agent::filesys::{dirs, Overwrite};
use miru_agent::models::{Deployment, DplActivity, Release, UploadRule, UploadRuleSource};
use miru_agent::scan::ScanErr;
use miru_agent::sync::syncer::SyncEvent;
use miru_agent::workers::scan_bridge;

// external crates
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

// =============================== TEST HELPERS ================================= //

/// Real disk stores (deployments / releases / upload_rules) backed by a temp
/// dir, seeded via `write_if_absent` — the same pattern as the
/// `disk/upload_rules.rs` inline tests.
struct Stores {
    _dir: dirs::TempDir,
    deployments: Arc<disk::Deployments>,
    releases: Arc<disk::Releases>,
    upload_rules: Arc<disk::UploadRules>,
}

impl Stores {
    async fn new() -> Self {
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        let (deployments, _) = disk::Deployments::spawn(64, layout.deployments(), 1000)
            .await
            .unwrap();
        let (releases, _) = disk::Releases::spawn(64, layout.releases(), 1000)
            .await
            .unwrap();
        let (upload_rules, _) = disk::UploadRules::spawn(64, layout.upload_rules(), 1000)
            .await
            .unwrap();
        Self {
            _dir: dir,
            deployments: Arc::new(deployments),
            releases: Arc::new(releases),
            upload_rules: Arc::new(upload_rules),
        }
    }

    /// Seed a Deployed deployment -> release -> upload rule bodies so the
    /// deployment-scoped traversal resolves to the given rules.
    async fn seed_deployed(&self, dpl_id: &str, release_id: &str, rules: &[UploadRule]) {
        self.deployments
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
        self.releases
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
            self.upload_rules
                .write_if_absent(rule.id.clone(), rule.clone(), |_, _| false)
                .await
                .unwrap();
        }
    }

    /// Seed a Queued (NOT Deployed) deployment: `find_deployed` returns None.
    async fn seed_queued(&self, dpl_id: &str, release_id: &str) {
        self.deployments
            .write_if_absent(
                dpl_id.to_string(),
                Deployment {
                    id: dpl_id.to_string(),
                    activity_status: DplActivity::Queued,
                    release_id: release_id.to_string(),
                    ..Default::default()
                },
                |_, _| false,
            )
            .await
            .unwrap();
    }

    /// Add an upload rule body (glob only) under `id`, leaving any existing
    /// release wiring untouched.
    async fn put_rule(&self, id: &str, glob: &str) {
        self.upload_rules
            .write_if_absent(id.to_string(), rule_with(id, glob, 0), |_, _| false)
            .await
            .unwrap();
    }

    /// Point the release at exactly `rule_ids`, overwriting whatever it
    /// referenced before.
    async fn set_release_rules(&self, release_id: &str, rule_ids: &[&str]) {
        self.releases
            .write(
                release_id.to_string(),
                Release {
                    id: release_id.to_string(),
                    upload_rule_ids: rule_ids.iter().map(|id| id.to_string()).collect(),
                    ..Default::default()
                },
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();
    }
}

/// Build an UploadRule with only its id + source fields set.
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

/// The set of rule ids of an update_rules call payload, order-independent.
fn rule_ids(rules: &[UploadRule]) -> BTreeSet<String> {
    rules.iter().map(|r| r.id.clone()).collect()
}

/// The expected rule-id set built from `&str` literals — complements
/// `rule_ids` (which reads a payload) for `assert_eq!` comparisons.
fn rule_id_set<const N: usize>(ids: [&str; N]) -> BTreeSet<String> {
    ids.iter().map(|id| id.to_string()).collect()
}

/// Poll `cond` until it returns true, yielding between attempts. Fails the
/// test (panics) after a bounded timeout so a broken worker can never hang
/// the suite. Avoids fixed sleeps that could flake under load.
async fn wait_until<F>(cond: F)
where
    F: Fn() -> bool,
{
    let poll = async {
        while !cond() {
            tokio::task::yield_now().await;
        }
    };
    tokio::time::timeout(Duration::from_secs(5), poll)
        .await
        .expect("condition not met within timeout");
}

/// Wait until the scanner has recorded `n` update_rules calls, then return
/// the nth call's rules payload. Fuses the `wait_until(len == n)` +
/// `[n - 1].1` idiom used by the resolution assertions.
async fn await_nth_update_rules(scanner: &MockScanner, n: usize) -> Vec<UploadRule> {
    wait_until(|| scanner.update_rules_calls().len() == n).await;
    scanner.update_rules_calls()[n - 1].1.clone()
}

/// Fire a SyncSuccess event on the syncer's watch channel; the worker
/// re-resolves from disk in response.
fn fire_sync_success(syncer: &MockSyncer) {
    syncer
        .get_transmitter()
        .send(SyncEvent::SyncSuccess)
        .unwrap();
}

/// Spawn the scan bridge worker against the given mocks/stores with a
/// oneshot-driven shutdown. Returns the run task handle and the shutdown
/// trigger; sending on the trigger fires the shutdown future.
fn spawn_bridge(
    scanner: Arc<MockScanner>,
    syncer: Arc<MockSyncer>,
    stores: Arc<Stores>,
) -> (JoinHandle<()>, oneshot::Sender<()>) {
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let shutdown_signal = Box::pin(async move {
        let _ = shutdown_rx.await;
    });
    let storage = scan_bridge::Storage {
        deployments: stores.deployments.clone(),
        releases: stores.releases.clone(),
        upload_rules: stores.upload_rules.clone(),
    };
    let handle = tokio::spawn(async move {
        scan_bridge::run(scanner.as_ref(), syncer.as_ref(), &storage, shutdown_signal).await;
    });
    (handle, shutdown_tx)
}

/// A running scan bridge worker plus the mocks/stores it drives. Built with
/// [`Bridge::start`] over an already-seeded `Arc<Stores>` — the worker does a
/// single startup disk read before its event loop, so seeding must precede
/// start. Dropping the fixture resolves the retained shutdown sender (which
/// fires the worker's shutdown select) and aborts the task as a backstop.
struct Bridge {
    stores: Arc<Stores>,
    scanner: Arc<MockScanner>,
    syncer: Arc<MockSyncer>,
    handle: JoinHandle<()>,
    _shutdown: oneshot::Sender<()>,
}

impl Bridge {
    /// Spawn the worker over the (already-seeded) stores. Seed the stores
    /// first: the worker reads disk once at startup, so spawning before
    /// seeding would race that read.
    async fn start(
        stores: Arc<Stores>,
        scanner: Arc<MockScanner>,
        syncer: Arc<MockSyncer>,
    ) -> Bridge {
        let (handle, shutdown) = spawn_bridge(scanner.clone(), syncer.clone(), stores.clone());
        Bridge {
            stores,
            scanner,
            syncer,
            handle,
            _shutdown: shutdown,
        }
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

// =============================== TESTS ======================================= //

pub mod run {
    use super::*;

    // Firing the shutdown future makes run() return: the spawned task's JoinHandle
    // completes.
    #[tokio::test]
    async fn shutdown_future_completes_run() {
        let stores = Arc::new(Stores::new().await);
        let r1 = rule_with("r1", "/none/*.mcap", 0);
        stores
            .seed_deployed("dpl", "rel", std::slice::from_ref(&r1))
            .await;

        let scanner = Arc::new(MockScanner::default());
        let syncer = Arc::new(MockSyncer::default());
        let (handle, shutdown_tx) = spawn_bridge(scanner.clone(), syncer.clone(), stores.clone());

        // let the startup seed run so the worker is in its event loop.
        wait_until(|| scanner.update_rules_calls().len() == 1).await;

        // fire shutdown; run() must return and the task must complete.
        shutdown_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("run did not return after shutdown fired")
            .expect("run task panicked");
    }
}

pub mod run_impl {
    use super::*;

    // At startup the worker seeds the scanner exactly once from the currently Deployed
    // deployment's rules, without waiting for any event.
    #[tokio::test]
    async fn startup_seeds_scanner_with_deployed_rules() {
        let stores = Arc::new(Stores::new().await);
        let r1 = rule_with("r1", "/none/*.mcap", 0);
        let r2 = rule_with("r2", "/none/*.mcap", 0);
        stores.seed_deployed("dpl", "rel", &[r1, r2]).await;

        let b = Bridge::start(
            stores,
            Arc::new(MockScanner::default()),
            Arc::new(MockSyncer::default()),
        )
        .await;

        let payload = await_nth_update_rules(&b.scanner, 1).await;
        assert_eq!(b.scanner.update_rules_calls()[0].0.id, "dpl");
        assert_eq!(rule_ids(&payload), rule_id_set(["r1", "r2"]));
        assert_eq!(b.scanner.clear_rules_calls(), 0);
    }

    // A SyncSuccess event re-resolves from disk: after mutating the deployed release to
    // reference a different rule set, the next update_rules reflects it.
    #[tokio::test]
    async fn sync_success_reresolves_changed_disk() {
        let stores = Arc::new(Stores::new().await);
        let r1 = rule_with("r1", "/none/*.mcap", 0);
        stores
            .seed_deployed("dpl", "rel", std::slice::from_ref(&r1))
            .await;

        let b = Bridge::start(
            stores,
            Arc::new(MockScanner::default()),
            Arc::new(MockSyncer::default()),
        )
        .await;

        // startup seed resolves to [r1].
        let payload = await_nth_update_rules(&b.scanner, 1).await;
        assert_eq!(rule_ids(&payload), rule_id_set(["r1"]));

        // add r2's body and point the release at [r1, r2].
        b.stores.put_rule("r2", "/none/*.mcap").await;
        b.stores.set_release_rules("rel", &["r1", "r2"]).await;

        // fire a SyncSuccess event; the worker re-resolves and pushes [r1, r2].
        fire_sync_success(&b.syncer);
        let payload = await_nth_update_rules(&b.scanner, 2).await;
        assert_eq!(rule_ids(&payload), rule_id_set(["r1", "r2"]));
    }
}

pub mod resolve_and_push {
    use super::*;

    // A Queued deployment clears scanner rules.
    #[tokio::test]
    async fn no_deployed_clears_rules() {
        let stores = Arc::new(Stores::new().await);
        stores.seed_queued("dpl", "rel").await;

        let b = Bridge::start(
            stores,
            Arc::new(MockScanner::default()),
            Arc::new(MockSyncer::default()),
        )
        .await;

        wait_until(|| b.scanner.clear_rules_calls() == 1).await;
        assert_eq!(b.scanner.update_rules_calls().len(), 0);

        // SyncSuccess clears again.
        fire_sync_success(&b.syncer);
        wait_until(|| b.scanner.clear_rules_calls() == 2).await;
        assert_eq!(b.scanner.update_rules_calls().len(), 0);
    }

    // A resolve error does not stop the worker.
    #[tokio::test]
    async fn resolve_error_worker_survives() {
        let stores = Arc::new(Stores::new().await);
        // Reference a missing release to force a resolve error.
        stores
            .deployments
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

        let b = Bridge::start(
            stores,
            Arc::new(MockScanner::default()),
            Arc::new(MockSyncer::default()),
        )
        .await;

        // The failed resolve does not change scanner rules.
        wait_until(|| true).await;
        assert_eq!(b.scanner.update_rules_calls().len(), 0);
        assert_eq!(b.scanner.clear_rules_calls(), 0);

        // Repair the release and verify the worker handles another sync.
        b.stores.put_rule("r1", "/none/*.mcap").await;
        b.stores.set_release_rules("absent", &["r1"]).await;
        fire_sync_success(&b.syncer);
        let payload = await_nth_update_rules(&b.scanner, 1).await;
        assert_eq!(rule_ids(&payload), rule_id_set(["r1"]));
    }

    // An update_rules error does not stop the worker.
    #[tokio::test]
    async fn update_rules_error_worker_survives() {
        let stores = Arc::new(Stores::new().await);
        let r1 = rule_with("r1", "/none/*.mcap", 0);
        stores
            .seed_deployed("dpl", "rel", std::slice::from_ref(&r1))
            .await;

        let scanner = Arc::new(MockScanner::default());
        // Fail the startup update.
        scanner.set_update_rules(|| {
            Err(ScanErr::InternalError(
                miru_agent::scan::errors::InternalError {
                    message: "boom".to_string(),
                    trace: miru_agent::trace!(),
                },
            ))
        });
        let b = Bridge::start(stores, scanner, Arc::new(MockSyncer::default())).await;

        // The failed update is recorded.
        wait_until(|| b.scanner.update_rules_calls().len() == 1).await;

        // Recover and handle another sync.
        b.scanner.set_update_rules(|| Ok(()));
        fire_sync_success(&b.syncer);
        wait_until(|| b.scanner.update_rules_calls().len() == 2).await;
    }

    // A clear_rules error does not stop the worker.
    #[tokio::test]
    async fn clear_rules_error_worker_survives() {
        let stores = Arc::new(Stores::new().await);
        stores.seed_queued("dpl", "rel").await;

        let scanner = Arc::new(MockScanner::default());
        // Fail the startup clear.
        scanner.set_clear_rules(|| {
            Err(ScanErr::InternalError(
                miru_agent::scan::errors::InternalError {
                    message: "boom".to_string(),
                    trace: miru_agent::trace!(),
                },
            ))
        });
        let b = Bridge::start(stores, scanner, Arc::new(MockSyncer::default())).await;

        // The failed clear is recorded.
        wait_until(|| b.scanner.clear_rules_calls() == 1).await;

        // Recover and handle another sync.
        b.scanner.set_clear_rules(|| Ok(()));
        fire_sync_success(&b.syncer);
        wait_until(|| b.scanner.clear_rules_calls() == 2).await;
        assert_eq!(b.scanner.update_rules_calls().len(), 0);
    }
}
