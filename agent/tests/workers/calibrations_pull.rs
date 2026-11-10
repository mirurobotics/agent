// standard crates
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::mocks::{
    http_client::{Call, MockClient},
    stub_token_manager::StubTokenManager,
    syncer::MockSyncer,
};
use crate::test_utils::{
    calibrations::{latest_store, miru_calibration, report},
    filesys::dirs as test_dirs,
    upload::token_manager,
};
use backend_api::models::CalibrationList;
use miru_agent::authn::{
    errors::{AuthnErr, MockError as AuthnMockError},
    TokenManagerExt,
};
use miru_agent::calibrations::{latest, Filter, LatestStore, ReportMetadata};
use miru_agent::http::{errors::MockErr, HTTPErr};
use miru_agent::sync::syncer::{SyncEvent, SyncFailure};
use miru_agent::workers::calibrations_pull;

// external crates
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// Fails a test that would otherwise hang.
const HANG_GUARD: Duration = Duration::from_secs(5);

/// A running worker over a mock syncer and Miru, and a real latest store.
struct Worker {
    _dir: test_dirs::TempDir,
    syncer: Arc<MockSyncer>,
    client: Arc<MockClient>,
    store: LatestStore,
    handle: JoinHandle<()>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl Worker {
    async fn start() -> Self {
        Self::start_with(Arc::new(MockSyncer::default()), token_manager()).await
    }

    async fn start_with<T>(syncer: Arc<MockSyncer>, token_mngr: Arc<T>) -> Self
    where
        T: TokenManagerExt + 'static,
    {
        let dir = test_dirs::temp("cal_pull_worker_test").unwrap();
        let store = latest_store(&dir, latest::DEFAULT_CAPACITY).await;
        let client = Arc::new(MockClient::default());
        let (shutdown, shutdown_rx) = oneshot::channel::<()>();
        let handle = {
            let (syncer, store, client) = (syncer.clone(), store.clone(), client.clone());
            tokio::spawn(async move {
                let shutdown_signal = Box::pin(async move {
                    let _ = shutdown_rx.await;
                });
                calibrations_pull::run(
                    syncer.as_ref(),
                    &store,
                    client.as_ref(),
                    token_mngr.as_ref(),
                    shutdown_signal,
                )
                .await;
            })
        };
        Self {
            _dir: dir,
            syncer,
            client,
            store,
            handle,
            shutdown: Some(shutdown),
        }
    }

    /// Miru serves the fixture calibration for `arm`.
    fn serve_arm(&self) {
        let list = CalibrationList {
            data: vec![miru_calibration("arm")],
            ..CalibrationList::default()
        };
        self.client
            .set_list_latest_calibrations(move || Ok(list.clone()));
        self.client
            .set_get_calibration_content(|id| Ok(format!("content of {id}")));
    }

    fn fail_list(&self) {
        self.client.set_list_latest_calibrations(|| {
            Err(HTTPErr::MockErr(MockErr {
                is_network_conn_err: false,
            }))
        });
    }

    fn fire(&self, event: SyncEvent) {
        self.syncer.get_transmitter().send(event).unwrap();
    }

    fn num_lists(&self) -> usize {
        self.client.call_count(Call::ListLatestCalibrations)
    }

    async fn entries(&self) -> Vec<ReportMetadata> {
        self.store.list(&Filter::default()).await.unwrap()
    }

    /// Wait until `n` lists of Miru's latest have been made.
    async fn await_lists(&self, n: usize) {
        guard(async {
            while self.num_lists() < n {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await;
    }

    /// Whether the worker is still running after a moment.
    async fn is_running(&mut self) -> bool {
        tokio::time::timeout(Duration::from_millis(50), &mut self.handle)
            .await
            .is_err()
    }

    /// Wait for the worker to return.
    async fn await_return(self) {
        guard(self.handle).await.unwrap();
    }

    async fn stop(mut self) {
        self.shutdown.take().unwrap().send(()).unwrap();
        self.await_return().await;
    }
}

async fn guard<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(HANG_GUARD, future)
        .await
        .expect("timed out")
}

#[tokio::test]
async fn pulls_after_a_successful_sync_and_returns() {
    let w = Worker::start().await;
    w.serve_arm();

    w.fire(SyncEvent::SyncSuccess);

    let entries = guard(async {
        loop {
            let entries = w.entries().await;
            if !entries.is_empty() {
                return entries;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert_eq!(entries, vec![report("arm").metadata]);
    w.await_return().await;
}

#[tokio::test]
async fn pulls_after_a_sync_that_ended_before_it_subscribed() {
    let syncer = Arc::new(MockSyncer::default());
    syncer
        .get_transmitter()
        .send(SyncEvent::SyncSuccess)
        .unwrap();

    let w = Worker::start_with(syncer, token_manager()).await;

    w.await_lists(1).await;
    w.await_return().await;
}

#[tokio::test]
async fn does_not_pull_after_a_failed_sync() {
    let mut w = Worker::start().await;

    w.fire(SyncEvent::SyncFailed(SyncFailure {
        is_network_conn_err: false,
    }));

    assert!(w.is_running().await);
    assert_eq!(w.num_lists(), 0);
    w.stop().await;
}

#[tokio::test]
async fn a_failed_pull_is_retried_after_the_next_successful_sync() {
    let mut w = Worker::start().await;
    w.fail_list();

    w.fire(SyncEvent::SyncSuccess);
    w.await_lists(1).await;
    assert!(w.is_running().await);

    w.serve_arm();
    w.fire(SyncEvent::SyncSuccess);
    w.await_lists(2).await;
    w.await_return().await;
}

#[tokio::test]
async fn a_failed_token_skips_the_pull_until_the_next_sync() {
    let err = AuthnErr::MockError(AuthnMockError {
        is_network_conn_err: false,
        trace: miru_agent::trace!(),
    });
    let token_mngr = Arc::new(StubTokenManager::err(err));
    let mut w = Worker::start_with(Arc::new(MockSyncer::default()), token_mngr).await;

    w.fire(SyncEvent::SyncSuccess);

    assert!(w.is_running().await);
    assert_eq!(w.num_lists(), 0);
    w.stop().await;
}

#[tokio::test]
async fn shutdown_stops_the_worker() {
    let mut w = Worker::start().await;

    assert!(w.is_running().await);
    w.stop().await;
}

#[tokio::test]
async fn returns_when_it_cannot_subscribe() {
    let syncer = Arc::new(MockSyncer::default());
    syncer.set_subscribe_fail(true);

    let w = Worker::start_with(syncer, token_manager()).await;

    w.await_return().await;
}

#[tokio::test]
async fn returns_when_the_sync_events_end() {
    let syncer = Arc::new(MockSyncer::default());
    syncer.drop_transmitter();

    let w = Worker::start_with(syncer, token_manager()).await;

    w.await_return().await;
}
