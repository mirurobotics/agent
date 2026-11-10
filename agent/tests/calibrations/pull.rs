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
        calibrations::pull_latest(
            &self.client,
            latest_cals_layout(&self.dir),
            capacity,
            "tok",
        )
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
}

#[tokio::test]
async fn overwrites_the_entry_already_there_for_a_key() {
    let fx = Fixture::new();
    let store = latest_store(&fx.dir, latest::DEFAULT_CAPACITY).await;
    store
        .put(reported("arm", None, None), "old content")
        .await
        .unwrap();
    store.shutdown().await.unwrap();
    fx.serve(&["arm"]);

    fx.pull(latest::DEFAULT_CAPACITY).await.unwrap();

    assert_eq!(fx.entries().await, vec![report("arm").metadata]);
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
