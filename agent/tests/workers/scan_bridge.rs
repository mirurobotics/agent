// standard crates
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::mocks::{scanner::MockScanner, syncer::MockSyncer};
use miru_agent::disk::{self, Layout};
use miru_agent::filesys::dirs;
use miru_agent::models::{Deployment, DplActivity, Release, UploadRule, UploadRuleSource};
use miru_agent::scan::ScanErr;
use miru_agent::sync::syncer::SyncEvent;
use miru_agent::workers::scan_bridge;

// external crates
use tokio::sync::oneshot;

pub mod run {
    use super::*;

    // =============================== TEST HELPERS ================================= //

    /// Real disk stores (deployments / releases / upload_rules) backed by a temp
    /// dir, seeded via `write_if_absent` — the same pattern as the
    /// `disk/upload_rules.rs` inline tests.
    struct Stores {
        _dir: dirs::TempDir,
        deployments: disk::Deployments,
        releases: disk::Releases,
        upload_rules: disk::UploadRules,
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
                deployments,
                releases,
                upload_rules,
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
    fn rule_ids(rules: &[UploadRule]) -> std::collections::BTreeSet<String> {
        rules.iter().map(|r| r.id.clone()).collect()
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

    /// Spawn the scan bridge worker against the given mocks/stores with a
    /// oneshot-driven shutdown. Returns the run task handle and the shutdown
    /// trigger; sending on the trigger fires the shutdown future.
    fn spawn_bridge(
        scanner: Arc<MockScanner>,
        syncer: Arc<MockSyncer>,
        stores: Arc<Stores>,
    ) -> (tokio::task::JoinHandle<()>, oneshot::Sender<()>) {
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let shutdown_signal = Box::pin(async move {
            let _ = shutdown_rx.await;
        });
        let handle = tokio::spawn(async move {
            scan_bridge::run(
                scanner.as_ref(),
                syncer.as_ref(),
                &stores.deployments,
                &stores.releases,
                &stores.upload_rules,
                shutdown_signal,
            )
            .await;
        });
        (handle, shutdown_tx)
    }

    // =============================== TESTS ======================================= //

    // (1) At startup the worker seeds the scanner exactly once from the currently
    // Deployed deployment's rules, without waiting for any event.
    #[tokio::test]
    async fn startup_seeds_scanner_with_deployed_rules() {
        let stores = Arc::new(Stores::new().await);
        let r1 = rule_with("r1", "/none/*.mcap", 0);
        let r2 = rule_with("r2", "/none/*.mcap", 0);
        stores.seed_deployed("dpl", "rel", &[r1, r2]).await;

        let scanner = Arc::new(MockScanner::default());
        let syncer = Arc::new(MockSyncer::default());
        let (_handle, _shutdown) = spawn_bridge(scanner.clone(), syncer.clone(), stores.clone());

        wait_until(|| scanner.update_rules_calls().len() == 1).await;
        let calls = scanner.update_rules_calls();
        assert_eq!(calls[0].0.id, "dpl");
        assert_eq!(
            rule_ids(&calls[0].1),
            std::collections::BTreeSet::from(["r1".to_string(), "r2".to_string()])
        );
        assert_eq!(scanner.clear_rules_calls(), 0);
    }

    // (2) A SyncSuccess event re-resolves from disk: after mutating the deployed
    // release to reference a different rule set, the next update_rules reflects it.
    #[tokio::test]
    async fn sync_success_reresolves_changed_disk() {
        let stores = Arc::new(Stores::new().await);
        let r1 = rule_with("r1", "/none/*.mcap", 0);
        stores
            .seed_deployed("dpl", "rel", std::slice::from_ref(&r1))
            .await;

        let scanner = Arc::new(MockScanner::default());
        let syncer = Arc::new(MockSyncer::default());
        let (_handle, _shutdown) = spawn_bridge(scanner.clone(), syncer.clone(), stores.clone());

        // startup seed resolves to [r1].
        wait_until(|| scanner.update_rules_calls().len() == 1).await;
        assert_eq!(
            rule_ids(&scanner.update_rules_calls()[0].1),
            std::collections::BTreeSet::from(["r1".to_string()])
        );

        // add r2's body and point the release at [r1, r2].
        let r2 = rule_with("r2", "/none/*.mcap", 0);
        stores
            .upload_rules
            .write_if_absent("r2".to_string(), r2, |_, _| false)
            .await
            .unwrap();
        stores
            .releases
            .write(
                "rel".to_string(),
                Release {
                    id: "rel".to_string(),
                    upload_rule_ids: vec!["r1".to_string(), "r2".to_string()],
                    ..Default::default()
                },
                |_, _| false,
                miru_agent::filesys::Overwrite::Allow,
            )
            .await
            .unwrap();

        // fire a SyncSuccess event; the worker re-resolves and pushes [r1, r2].
        syncer
            .get_transmitter()
            .send(SyncEvent::SyncSuccess)
            .unwrap();
        wait_until(|| scanner.update_rules_calls().len() == 2).await;
        assert_eq!(
            rule_ids(&scanner.update_rules_calls()[1].1),
            std::collections::BTreeSet::from(["r1".to_string(), "r2".to_string()])
        );
    }

    // (3) With no Deployed deployment (only a Queued one), the worker clears the
    // scanner rules and never calls update_rules.
    #[tokio::test]
    async fn no_deployed_clears_rules() {
        let stores = Arc::new(Stores::new().await);
        stores.seed_queued("dpl", "rel").await;

        let scanner = Arc::new(MockScanner::default());
        let syncer = Arc::new(MockSyncer::default());
        let (_handle, _shutdown) = spawn_bridge(scanner.clone(), syncer.clone(), stores.clone());

        wait_until(|| scanner.clear_rules_calls() == 1).await;
        assert_eq!(scanner.update_rules_calls().len(), 0);
    }

    // (4) A failing update_rules is logged, not propagated: the worker survives to
    // handle the next SyncSuccess event, producing a second (successful) call.
    #[tokio::test]
    async fn update_rules_error_is_logged_and_worker_survives() {
        let stores = Arc::new(Stores::new().await);
        let r1 = rule_with("r1", "/none/*.mcap", 0);
        stores
            .seed_deployed("dpl", "rel", std::slice::from_ref(&r1))
            .await;

        let scanner = Arc::new(MockScanner::default());
        // first update_rules errors; flip to Ok before the next event.
        scanner.set_update_rules(|| {
            Err(ScanErr::InternalError(
                miru_agent::scan::errors::InternalError {
                    message: "boom".to_string(),
                    trace: miru_agent::trace!(),
                },
            ))
        });
        let syncer = Arc::new(MockSyncer::default());
        let (_handle, _shutdown) = spawn_bridge(scanner.clone(), syncer.clone(), stores.clone());

        // startup seed: the update_rules call happened but returned Err (logged).
        wait_until(|| scanner.update_rules_calls().len() == 1).await;

        // recover: subsequent calls succeed, and a new event is still handled.
        scanner.set_update_rules(|| Ok(()));
        syncer
            .get_transmitter()
            .send(SyncEvent::SyncSuccess)
            .unwrap();
        wait_until(|| scanner.update_rules_calls().len() == 2).await;
    }

    // (5) Firing the shutdown future makes run() return: the spawned task's
    // JoinHandle completes.
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
