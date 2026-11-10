//! Pulling the control plane's latest calibrations into a latest calibrations
//! store.

// standard crates
use std::sync::{Arc, Mutex};
use std::time::Duration;

// internal crates
use crate::mocks::http_client::{Call, MockClient};
use crate::test_utils::calibrations::{
    latest_cals_layout, latest_store, miru_calibration, report, report_id, reported,
};
use crate::test_utils::filesys::dirs as test_dirs;
use crate::test_utils::upload::token_manager;
use backend_api::models::CalibrationList;
use miru_agent::authn::errors::{AuthnErr, MockError as AuthnMockError};
use miru_agent::calibrations::{self, latest, CalibrationsErr, Filter, Report, ReportMetadata};
use miru_agent::filesys::{files, WriteOptions};
use miru_agent::http::{errors::MockErr, HTTPErr};

// external crates
use chrono::TimeDelta;
use tokio::time::Instant;

struct Fixture {
    dir: test_dirs::TempDir,
    client: MockClient,
}

impl Fixture {
    fn new() -> Self {
        Self {
            dir: test_dirs::temp("pull_latest_test").unwrap(),
            client: MockClient::default(),
        }
    }

    /// The control plane serves the fixture calibration for each of `slugs`,
    /// and a content named by its calibration's id.
    fn serve(&self, slugs: &[&str]) {
        let list = CalibrationList {
            data: slugs.iter().map(|slug| miru_calibration(slug)).collect(),
            ..CalibrationList::default()
        };
        self.client
            .set_list_latest_calibrations(move || Ok(list.clone()));
        self.client
            .set_get_calibration_content(|id| Ok(content(id)));
    }

    async fn pull(&self, capacity: usize) -> Result<(), CalibrationsErr> {
        let store = latest_store(&self.dir, capacity).await;
        calibrations::pull_latest(&store, &self.client, "tok").await
    }

    /// Every entry in the store, in key order.
    async fn entries(&self) -> Vec<ReportMetadata> {
        let store = latest_store(&self.dir, latest::DEFAULT_CAPACITY).await;
        store.list(&Filter::default()).await.unwrap()
    }
}

/// The content the control plane serves for the calibration `id`.
fn content(id: &str) -> String {
    format!("content of {id}")
}

fn mock_err() -> HTTPErr {
    HTTPErr::MockErr(MockErr {
        is_network_conn_err: false,
    })
}

#[tokio::test]
async fn writes_the_control_planes_latest_calibrations_into_the_given_store() {
    let fx = Fixture::new();
    fx.serve(&["arm", "leg"]);

    fx.pull(latest::DEFAULT_CAPACITY).await.unwrap();

    let expected = vec![report("arm").metadata, report("leg").metadata];
    assert_eq!(fx.entries().await, expected);
    let store = latest_store(&fx.dir, latest::DEFAULT_CAPACITY).await;
    let found = store.get(report_id("arm")).await.unwrap();
    let expected = Report {
        metadata: report("arm").metadata,
        data: content("cal_arm"),
    };
    assert_eq!(found, expected);
    assert_eq!(fx.client.call_count(Call::GetCalibrationContent), 2);
    let requests = fx.client.requests();
    assert!(requests.iter().all(|r| r.token.as_deref() == Some("tok")));
    let expand = ("expand".to_string(), "reported_by".to_string());
    assert!(requests[0].query.contains(&expand));
}

/// Seed the store with `entries`, each with content named by its component.
async fn seed(fx: &Fixture, entries: &[ReportMetadata]) {
    let store = latest_store(&fx.dir, latest::DEFAULT_CAPACITY).await;
    for entry in entries {
        let data = format!("device's {}", entry.component);
        store.put(entry.clone(), &data).await.unwrap();
    }
    store.shutdown().await.unwrap();
}

/// A report for `component` reported `secs` after the control plane's for that key.
fn reported_after(component: &str, secs: i64) -> ReportMetadata {
    let metadata = reported(component, None, None);
    ReportMetadata {
        reported_at: metadata.reported_at + TimeDelta::seconds(secs),
        ..metadata
    }
}

#[tokio::test]
async fn replaces_an_older_entry_for_a_key() {
    let fx = Fixture::new();
    seed(&fx, &[reported_after("arm", -1)]).await;
    fx.serve(&["arm"]);

    fx.pull(latest::DEFAULT_CAPACITY).await.unwrap();

    assert_eq!(fx.entries().await, vec![report("arm").metadata]);
}

#[tokio::test]
async fn keeps_an_entry_as_new_or_newer_without_downloading() {
    let fx = Fixture::new();
    let stored = vec![reported_after("arm", 0), reported_after("leg", 1)];
    seed(&fx, &stored).await;
    fx.serve(&["arm", "leg"]);

    fx.pull(latest::DEFAULT_CAPACITY).await.unwrap();

    assert_eq!(fx.entries().await, stored);
    assert_eq!(fx.client.call_count(Call::GetCalibrationContent), 0);
}

#[tokio::test]
async fn keeps_keys_the_control_plane_does_not_have() {
    let fx = Fixture::new();
    let leg = reported("leg", None, None);
    seed(&fx, std::slice::from_ref(&leg)).await;
    fx.serve(&["arm"]);

    fx.pull(latest::DEFAULT_CAPACITY).await.unwrap();

    assert_eq!(fx.entries().await, vec![report("arm").metadata, leg]);
}

#[tokio::test]
async fn writes_nothing_when_the_control_plane_is_empty() {
    let fx = Fixture::new();
    fx.serve(&[]);

    fx.pull(latest::DEFAULT_CAPACITY).await.unwrap();

    assert!(fx.entries().await.is_empty());
    assert_eq!(fx.client.call_count(Call::GetCalibrationContent), 0);
}

#[tokio::test]
async fn more_keys_than_capacity_fail_the_call() {
    let fx = Fixture::new();
    fx.serve(&["arm", "leg"]);

    let result = fx.pull(1).await;

    assert!(matches!(result, Err(CalibrationsErr::StoreFullErr(_))));
}

#[tokio::test]
async fn an_unreadable_store_fails_the_call() {
    let fx = Fixture::new();
    let index = latest_cals_layout(&fx.dir).index();
    files::write_string(&index, "not json", WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();
    fx.serve(&["arm"]);

    let result = fx.pull(latest::DEFAULT_CAPACITY).await;

    assert!(matches!(result, Err(CalibrationsErr::FileSysErr(_))));
    assert_eq!(fx.client.call_count(Call::GetCalibrationContent), 0);
}

#[tokio::test]
async fn a_failed_list_fails_the_call() {
    let fx = Fixture::new();
    fx.client.set_list_latest_calibrations(|| Err(mock_err()));

    let result = fx.pull(latest::DEFAULT_CAPACITY).await;

    assert!(matches!(result, Err(CalibrationsErr::HTTPErr(_))));
    assert_eq!(fx.client.call_count(Call::GetCalibrationContent), 0);
}

#[tokio::test]
async fn a_failed_download_fails_the_call() {
    let fx = Fixture::new();
    fx.serve(&["arm", "leg"]);
    fx.client.set_get_calibration_content(|id| match id {
        "cal_leg" => Err(mock_err()),
        _ => Ok(content(id)),
    });

    let result = fx.pull(latest::DEFAULT_CAPACITY).await;

    assert!(matches!(result, Err(CalibrationsErr::HTTPErr(_))));
}

const NETWORK: bool = true;
const OTHER: bool = false;

/// The control plane fails a list for each of `failures`, with a network error
/// where it is `NETWORK`, and then serves `arm`. Returns when each list came.
fn fail_lists(fx: &Fixture, failures: &[bool]) -> Arc<Mutex<Vec<Instant>>> {
    let failures = failures.to_vec();
    let list = CalibrationList {
        data: vec![miru_calibration("arm")],
        ..CalibrationList::default()
    };
    let times = Arc::new(Mutex::new(Vec::new()));
    let listed = times.clone();
    fx.client.set_list_latest_calibrations(move || {
        let mut listed = listed.lock().unwrap();
        listed.push(Instant::now());
        match failures.get(listed.len() - 1) {
            Some(&is_network_conn_err) => Err(HTTPErr::MockErr(MockErr {
                is_network_conn_err,
            })),
            None => Ok(list.clone()),
        }
    });
    fx.client.set_get_calibration_content(|id| Ok(content(id)));
    times
}

/// The waits between consecutive `times`.
fn gaps(times: &Mutex<Vec<Instant>>) -> Vec<Duration> {
    let times = times.lock().unwrap();
    times.windows(2).map(|pair| pair[1] - pair[0]).collect()
}

/// Run `pull_latest_until_success` into a fresh store and return its entries.
async fn pull_until_success(fx: &Fixture) -> Vec<ReportMetadata> {
    let store = latest_store(&fx.dir, latest::DEFAULT_CAPACITY).await;
    calibrations::pull_latest_until_success(&store, &fx.client, token_manager().as_ref()).await;
    store.list(&Filter::default()).await.unwrap()
}

#[tokio::test(start_paused = true)]
async fn pull_latest_until_success_doubles_its_wait_after_each_failure() {
    let fx = Fixture::new();
    let times = fail_lists(&fx, &[OTHER; 3]);

    let entries = pull_until_success(&fx).await;

    assert_eq!(entries, vec![report("arm").metadata]);
    assert_eq!(gaps(&times), [10, 20, 40].map(Duration::from_secs));
}

#[tokio::test(start_paused = true)]
async fn pull_latest_until_success_waits_at_most_an_hour() {
    let fx = Fixture::new();
    let times = fail_lists(&fx, &[OTHER; 11]);

    pull_until_success(&fx).await;

    let gaps = gaps(&times);
    assert_eq!(gaps[8..], [2560, 3600, 3600].map(Duration::from_secs));
}

#[tokio::test(start_paused = true)]
async fn pull_latest_until_success_waits_a_flat_ten_seconds_after_network_errors() {
    let fx = Fixture::new();
    // each pull lists three times on a network error, so this fails three pulls
    let times = fail_lists(&fx, &[NETWORK; 9]);

    let entries = pull_until_success(&fx).await;

    assert_eq!(entries, vec![report("arm").metadata]);
    let between_pulls: Vec<_> = gaps(&times).into_iter().skip(2).step_by(3).collect();
    assert_eq!(between_pulls, [10, 10, 10].map(Duration::from_secs));
}

#[tokio::test(start_paused = true)]
async fn pull_latest_until_success_does_not_count_network_errors() {
    let fx = Fixture::new();
    let times = fail_lists(&fx, &[NETWORK, NETWORK, NETWORK, OTHER]);

    pull_until_success(&fx).await;

    // the failure after a network-failed pull still waits the first backoff
    assert_eq!(gaps(&times)[2..], [10, 10].map(Duration::from_secs));
}

#[tokio::test(start_paused = true)]
async fn pull_latest_until_success_tries_again_after_a_token_failure() {
    let fx = Fixture::new();
    fx.serve(&["arm"]);
    let token_mngr = token_manager();
    token_mngr.fail_next_get_token(AuthnErr::MockError(AuthnMockError {
        is_network_conn_err: false,
        trace: miru_agent::trace!(),
    }));
    let store = latest_store(&fx.dir, latest::DEFAULT_CAPACITY).await;

    calibrations::pull_latest_until_success(&store, &fx.client, token_mngr.as_ref()).await;

    let entries = store.list(&Filter::default()).await.unwrap();
    assert_eq!(entries, vec![report("arm").metadata]);
    assert_eq!(token_mngr.num_get_token_calls(), 2);
}
