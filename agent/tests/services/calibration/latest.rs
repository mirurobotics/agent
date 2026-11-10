//! Listing the latest calibrations that intake wrote through or the puller
//! pulled.

// internal crates
use crate::services::calibration::helpers::{params, Fixture};
use crate::test_utils::calibrations::{miru_calibration, received_at, report};
use backend_api::models::CalibrationList;
use miru_agent::calibrations::{Filter, ReportMetadata};
use miru_agent::models::InstanceFormat;
use miru_agent::services::calibration::{self, CreateParams};
use miru_agent::services::ServiceErr;

// external crates
use uuid::Uuid;

#[tokio::test]
async fn list_latest_includes_a_created_report() {
    let f = Fixture::new().await;
    let created = f.create(params()).await.unwrap();

    let latest = calibration::list_latest(&f.latest, &f.puller, &Filter::default()).await;

    let metadata = ReportMetadata {
        report_id: created.report_id,
        component: "arm".to_string(),
        format: InstanceFormat::Yaml,
        role: Some("left".to_string()),
        aspect: Some("pose".to_string()),
        serial_number: Some("SN-1".to_string()),
        reported_by: Some("Ada Lovelace".to_string()),
        deployment_id: None,
        reported_at: received_at(),
    };
    assert_eq!(latest.unwrap(), [metadata]);
}

#[tokio::test]
async fn list_latest_pulls_the_control_planes_latest_first() {
    let f = Fixture::new().await;
    let list = CalibrationList {
        data: vec![miru_calibration("arm")],
        ..CalibrationList::default()
    };
    f.client.set_list_latest_calibrations(move || Ok(list.clone()));

    let latest = calibration::list_latest(&f.latest, &f.puller, &Filter::default()).await;

    assert_eq!(latest.unwrap(), [report("arm").metadata]);
}

#[tokio::test]
async fn list_latest_applies_the_filter() {
    let f = Fixture::new().await;
    f.create(params()).await.unwrap();
    let leg = CreateParams {
        component: "leg".to_string(),
        ..params()
    };
    let created = f.create(leg).await.unwrap();
    let filter = Filter {
        component: vec!["leg".to_string()],
        ..Filter::default()
    };

    let latest = calibration::list_latest(&f.latest, &f.puller, &filter).await;

    let listed = latest.unwrap().into_iter();
    let ids: Vec<Uuid> = listed.map(|metadata| metadata.report_id).collect();
    assert_eq!(ids, [created.report_id]);
}

#[tokio::test]
async fn list_latest_fails_when_the_store_cannot_be_read() {
    let f = Fixture::new().await;
    f.corrupt_latest_index().await;

    let result = calibration::list_latest(&f.latest, &f.puller, &Filter::default()).await;

    let err = result.unwrap_err();
    assert!(matches!(err, ServiceErr::CalibrationsErr(_)), "{err:?}");
}
