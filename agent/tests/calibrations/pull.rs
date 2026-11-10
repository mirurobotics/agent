//! Pulling Miru's latest calibrations into a latest calibrations store.

// internal crates
use crate::mocks::http_client::{Call, MockClient};
use crate::test_utils::calibrations::{
    latest_cals_layout, latest_store, miru_calibration, report, report_id, reported,
};
use crate::test_utils::filesys::dirs as test_dirs;
use backend_api::models::CalibrationList;
use miru_agent::calibrations::{self, latest, CalibrationsErr, Filter, Report, ReportMetadata};
use miru_agent::http::{errors::MockErr, HTTPErr};

// external crates
use chrono::TimeDelta;

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

    /// Miru serves the fixture calibration for each of `slugs`, and a content
    /// named by its calibration's id.
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
        calibrations::pull_latest(&self.client, latest_cals_layout(&self.dir), capacity, "tok")
            .await
    }

    /// Every entry in the store, in key order.
    async fn entries(&self) -> Vec<ReportMetadata> {
        let store = latest_store(&self.dir, latest::DEFAULT_CAPACITY).await;
        store.list(&Filter::default()).await.unwrap()
    }
}

/// The content Miru serves for the calibration `id`.
fn content(id: &str) -> String {
    format!("content of {id}")
}

fn mock_err() -> HTTPErr {
    HTTPErr::MockErr(MockErr {
        is_network_conn_err: false,
    })
}

#[tokio::test]
async fn writes_mirus_latest_calibrations_into_the_given_store() {
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

/// A report for `component` reported `secs` after Miru's for that key.
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
async fn keeps_keys_miru_does_not_have() {
    let fx = Fixture::new();
    let leg = reported("leg", None, None);
    seed(&fx, std::slice::from_ref(&leg)).await;
    fx.serve(&["arm"]);

    fx.pull(latest::DEFAULT_CAPACITY).await.unwrap();

    assert_eq!(fx.entries().await, vec![report("arm").metadata, leg]);
}

#[tokio::test]
async fn writes_nothing_when_miru_is_empty() {
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
