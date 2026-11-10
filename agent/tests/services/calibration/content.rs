//! Reading a latest calibration's content by its `report_id`.

// internal crates
use crate::services::calibration::helpers::{params, Fixture};
use miru_agent::calibrations::{CalibrationsErr, LatestContent};
use miru_agent::models::InstanceFormat;
use miru_agent::services::calibration;
use miru_agent::services::ServiceErr;

// external crates
use uuid::Uuid;

#[tokio::test]
async fn get_content_returns_the_reported_data() {
    let f = Fixture::new().await;
    let created = f.create(params()).await.unwrap();
    let report_id = created.report_id.to_string();

    let content = calibration::get_content(&f.latest, &report_id).await;

    let want = LatestContent {
        report_id: created.report_id,
        format: InstanceFormat::Yaml,
        data: params().data,
    };
    assert_eq!(content.unwrap(), want);
}

#[tokio::test]
async fn get_content_of_an_unknown_report_id_is_not_found() {
    let f = Fixture::new().await;
    f.create(params()).await.unwrap();
    let report_id = Uuid::new_v4().to_string();

    let result = calibration::get_content(&f.latest, &report_id).await;

    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            ServiceErr::CalibrationsErr(CalibrationsErr::NotFoundErr(_))
        ),
        "{err:?}"
    );
}

#[tokio::test]
async fn get_content_of_an_invalid_report_id_is_not_found() {
    let f = Fixture::new().await;

    let result = calibration::get_content(&f.latest, "not-a-uuid").await;

    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            ServiceErr::CalibrationsErr(CalibrationsErr::NotFoundErr(_))
        ),
        "{err:?}"
    );
}
