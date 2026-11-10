//! Pulling Miru's latest calibrations into a latest calibrations store.

// internal crates
use crate::mocks::http_client::{Call, MockClient};
use crate::test_utils::calibrations::{
    latest_cals_layout, latest_store, layout, miru_calibration, report, report_id, reported,
};
use crate::test_utils::filesys::dirs as test_dirs;
use backend_api::models::CalibrationList;
use miru_agent::calibrations::{
    self, latest, CalibrationsErr, Filter, QueueEntry, QueueSnapshot, Report, ReportMetadata,
};
use miru_agent::filesys::{files, WriteOptions};
use miru_agent::http::{errors::MockErr, HTTPErr};

// external crates
use chrono::TimeDelta;
use uuid::Uuid;

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
async fn keeps_an_entry_reported_at_the_same_time_or_later() {
    let fx = Fixture::new();
    let stored = vec![reported_after("arm", 0), reported_after("leg", 1)];
    seed(&fx, &stored).await;
    fx.serve(&["arm", "leg"]);

    fx.pull(latest::DEFAULT_CAPACITY).await.unwrap();

    assert_eq!(fx.entries().await, stored);
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

// ============================= MARK AND PULL ============================ //
#[tokio::test]
async fn mark_needs_pull_marks_the_store() {
    let fx = Fixture::new();

    calibrations::mark_needs_pull(latest_cals_layout(&fx.dir))
        .await
        .unwrap();

    let store = latest_store(&fx.dir, latest::DEFAULT_CAPACITY).await;
    assert!(store.needs_pull().await.unwrap());
}

#[tokio::test]
async fn pull_if_needed_pulls_and_unmarks_a_marked_store() {
    let fx = Fixture::new();
    let store = latest_store(&fx.dir, latest::DEFAULT_CAPACITY).await;
    store.set_needs_pull(true).await.unwrap();
    fx.serve(&["arm"]);

    calibrations::pull_if_needed(&store, &fx.client, "tok")
        .await
        .unwrap();

    assert_eq!(fx.entries().await, vec![report("arm").metadata]);
    assert!(!store.needs_pull().await.unwrap());
}

#[tokio::test]
async fn pull_if_needed_does_nothing_for_an_unmarked_store() {
    let fx = Fixture::new();
    let store = latest_store(&fx.dir, latest::DEFAULT_CAPACITY).await;
    fx.serve(&["arm"]);

    calibrations::pull_if_needed(&store, &fx.client, "tok")
        .await
        .unwrap();

    assert!(fx.client.requests().is_empty());
    assert!(fx.entries().await.is_empty());
}

#[tokio::test]
async fn a_failed_pull_if_needed_keeps_the_mark() {
    let fx = Fixture::new();
    let store = latest_store(&fx.dir, latest::DEFAULT_CAPACITY).await;
    store.set_needs_pull(true).await.unwrap();
    fx.client.set_list_latest_calibrations(|| Err(mock_err()));

    let result = calibrations::pull_if_needed(&store, &fx.client, "tok").await;

    assert!(matches!(result, Err(CalibrationsErr::HTTPErr(_))));
    assert!(store.needs_pull().await.unwrap());
}

// ============================ MERGE QUEUED ============================== //
/// Queue each of `reports` in the fixture's upload queue, with a report file
/// holding `data` for each except those named in `without_data`.
async fn queue(fx: &Fixture, reports: &[ReportMetadata], without_data: &[&str]) {
    let cals = layout(&fx.dir);
    let opts = WriteOptions::OVERWRITE_ATOMIC;
    let mut snapshot = QueueSnapshot::default();
    for metadata in reports {
        let entry = QueueEntry {
            id: Uuid::new_v4(),
            job: metadata.clone(),
            attempts: 0,
            next_attempt_at: None,
        };
        if !without_data.contains(&metadata.component.as_str()) {
            let file = cals.reports().file(&entry.id.to_string());
            let data = format!("queued {}", metadata.component);
            files::write_string(&file, &data, opts).await.unwrap();
        }
        snapshot.entries.push(entry);
    }
    files::write_json(&cals.queue(), &snapshot, opts)
        .await
        .unwrap();
}

#[tokio::test]
async fn merge_queued_puts_queued_reports_newer_than_the_store() {
    let fx = Fixture::new();
    let arm = reported_after("arm", 1);
    let leg = reported("leg", None, None);
    seed(&fx, &[reported("arm", None, None)]).await;
    queue(&fx, &[arm.clone(), leg.clone()], &[]).await;

    calibrations::merge_queued(&layout(&fx.dir)).await.unwrap();

    assert_eq!(fx.entries().await, vec![arm.clone(), leg]);
    let store = latest_store(&fx.dir, latest::DEFAULT_CAPACITY).await;
    let found = store.get(arm.report_id).await.unwrap();
    assert_eq!(found.data, "queued arm");
}

#[tokio::test]
async fn merge_queued_keeps_a_stored_entry_as_new_or_newer() {
    let fx = Fixture::new();
    let stored = reported_after("arm", 1);
    seed(&fx, std::slice::from_ref(&stored)).await;
    queue(&fx, &[reported("arm", None, None)], &[]).await;

    calibrations::merge_queued(&layout(&fx.dir)).await.unwrap();

    assert_eq!(fx.entries().await, vec![stored]);
}

#[tokio::test]
async fn merge_queued_skips_a_report_without_data() {
    let fx = Fixture::new();
    let arm = reported("arm", None, None);
    let leg = reported("leg", None, None);
    queue(&fx, &[arm, leg.clone()], &["arm"]).await;

    calibrations::merge_queued(&layout(&fx.dir)).await.unwrap();

    assert_eq!(fx.entries().await, vec![leg]);
}

#[tokio::test]
async fn merge_queued_does_nothing_without_a_queue() {
    let fx = Fixture::new();

    calibrations::merge_queued(&layout(&fx.dir)).await.unwrap();

    assert!(fx.entries().await.is_empty());
}
