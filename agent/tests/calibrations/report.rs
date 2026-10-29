//! The calibration queue: the generic suite instantiated for `ReportMetadata`,
//! plus the on-disk wire pin for `calibrations/queue.json`.

// internal crates
use crate::queue::file::queue_suite;
use crate::test_utils::calibrations::report_id;
use crate::test_utils::filesys::dirs as test_dirs;
use miru_agent::calibrations::{Queue, QueueSnapshot, QueueSnapshotFile, ReportMetadata};
use miru_agent::filesys::{files, File, WriteOptions};
use miru_agent::models::InstanceFormat;
use miru_agent::queue::{Persistence, QueueJob};

// external crates
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Deterministic report metadata, ready the moment it arrives.
fn metadata(name: &str) -> ReportMetadata {
    ReportMetadata {
        report_id: report_id(name),
        component: name.to_string(),
        format: InstanceFormat::Yaml,
        role: Some("aligner-1".to_string()),
        aspect: Some("pose".to_string()),
        serial_number: Some("SN-1".to_string()),
        user: Some("Ada Lovelace".to_string()),
        deployment_id: Some("dpl_1".to_string()),
        reported_at: DateTime::from_timestamp(1000, 0).unwrap(),
    }
}

queue_suite!(metadata, "calibration-generic-queue-test");

#[test]
fn ready_at_is_min_utc() {
    assert_eq!(
        QueueJob::ready_at(&metadata("arm")),
        DateTime::<Utc>::MIN_UTC
    );
}

#[test]
fn name_is_the_component_role_aspect_and_time() {
    assert_eq!(
        "calibration of arm (role aligner-1) (aspect pose) received at 1970-01-01T00:16:40+00:00",
        QueueJob::name(&metadata("arm"))
    );
}

#[test]
fn name_leaves_out_a_missing_role_and_aspect() {
    let report = ReportMetadata {
        role: None,
        aspect: None,
        ..metadata("arm")
    };

    assert_eq!(
        "calibration of arm received at 1970-01-01T00:16:40+00:00",
        QueueJob::name(&report)
    );
}

async fn open(path: &File) -> QueueSnapshotFile {
    QueueSnapshotFile::new_with_default(path.clone(), QueueSnapshot::default())
        .await
        .unwrap()
}

mod wire {
    use super::*;

    /// Pins the persisted shape of `queue.json`. It holds queued reports the
    /// agent has promised to deliver, so a shape change must fail here rather
    /// than strand a live queue.
    #[tokio::test]
    async fn raw_json_snapshot_loads() {
        let dir = test_dirs::temp("calibration_queue_test").unwrap();
        let path = dir.to_dir().file("queue.json");
        let id = Uuid::new_v4();
        let raw = format!(
            concat!(
                r#"{{"entries":[{{"id":"{id}","job":{{"#,
                r#""report_id":"{report_id}","component":"arm","format":"yaml","#,
                r#""role":"aligner-1","aspect":"pose","serial_number":"SN-1","#,
                r#""user":"Ada Lovelace","#,
                r#""deployment_id":"dpl_1","reported_at":"1970-01-01T00:16:40Z""#,
                r#"}},"attempts":0,"next_attempt_at":null}}]}}"#,
            ),
            id = id,
            report_id = report_id("arm"),
        );
        files::write_string(&path, &raw, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let queue = Queue::from_snapshot(8, open(&path).await, Persistence::BestEffort);

        let entry = queue.next_ready(Utc::now()).unwrap();
        assert_eq!(entry.id, id);
        assert_eq!(entry.job, metadata("arm"));
    }

    /// The optional fields may be absent, as for a calibration taken with no
    /// deployment, role, aspect, serial number or user.
    #[tokio::test]
    async fn optional_fields_default_to_none() {
        let dir = test_dirs::temp("calibration_queue_test").unwrap();
        let path = dir.to_dir().file("queue.json");
        let raw = format!(
            concat!(
                r#"{{"entries":[{{"job":{{"report_id":"{report_id}","#,
                r#""component":"arm","format":"yaml","#,
                r#""reported_at":"1970-01-01T00:16:40Z"}}}}]}}"#,
            ),
            report_id = report_id("arm"),
        );
        files::write_string(&path, &raw, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let queue = Queue::from_snapshot(8, open(&path).await, Persistence::BestEffort);

        let found = queue.next_ready(Utc::now()).unwrap().job;
        let expected = ReportMetadata {
            role: None,
            aspect: None,
            serial_number: None,
            user: None,
            deployment_id: None,
            ..metadata("arm")
        };
        assert_eq!(found, expected);
    }

    /// Reports queued before `report_id` was part of the metadata are minted
    /// one on load.
    #[tokio::test]
    async fn a_missing_report_id_is_minted() {
        let dir = test_dirs::temp("calibration_queue_test").unwrap();
        let path = dir.to_dir().file("queue.json");
        let raw = concat!(
            r#"{"entries":[{"job":{"component":"arm","format":"yaml","#,
            r#""role":"aligner-1","aspect":"pose","serial_number":"SN-1","#,
            r#""deployment_id":"dpl_1","reported_at":"1970-01-01T00:16:40Z"}}]}"#,
        );
        files::write_string(&path, raw, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let queue = Queue::from_snapshot(8, open(&path).await, Persistence::BestEffort);

        let found = queue.next_ready(Utc::now()).unwrap().job;
        assert!(!found.report_id.is_nil());
        let expected = ReportMetadata {
            report_id: found.report_id,
            ..metadata("arm")
        };
        assert_eq!(found, expected);
    }
}
