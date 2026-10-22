//! The calibration queue: the generic suite instantiated for `ReportMetadata`,
//! plus the on-disk wire pin for `calibrations/queue.json`.

// internal crates
use crate::queue::file::queue_suite;
use crate::test_utils::filesys::dirs as test_dirs;
use miru_agent::calibrations::{Queue, QueueSnapshot, QueueSnapshotFile, ReportMetadata};
use miru_agent::filesys::{files, File, WriteOptions};
use miru_agent::models::InstanceFormat;
use miru_agent::queue::{Persistence, QueueJob};

// external crates
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Deterministic report metadata, due the moment it arrives.
fn metadata(name: &str) -> ReportMetadata {
    ReportMetadata {
        component_slug: name.to_string(),
        format: InstanceFormat::Yaml,
        role: Some("aligner-1".to_string()),
        aspect: Some("pose".to_string()),
        serial_number: Some("SN-1".to_string()),
        deployment_id: Some("dpl_1".to_string()),
        calibrated_at: DateTime::from_timestamp(1000, 0).unwrap(),
    }
}

queue_suite!(metadata, "calibration-generic-queue-test");

#[test]
fn due_at_is_min_utc() {
    assert_eq!(QueueJob::due_at(&metadata("arm")), DateTime::<Utc>::MIN_UTC);
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
                r#""component_slug":"arm","format":"yaml","#,
                r#""role":"aligner-1","aspect":"pose","serial_number":"SN-1","#,
                r#""deployment_id":"dpl_1","calibrated_at":"1970-01-01T00:16:40Z""#,
                r#"}},"attempts":0,"next_attempt_at":null}}]}}"#,
            ),
            id = id,
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
    /// deployment, role, aspect or serial number.
    #[tokio::test]
    async fn optional_fields_default_to_none() {
        let dir = test_dirs::temp("calibration_queue_test").unwrap();
        let path = dir.to_dir().file("queue.json");
        let raw = concat!(
            r#"{"entries":[{"job":{"component_slug":"arm","format":"yaml","#,
            r#""calibrated_at":"1970-01-01T00:16:40Z"}}]}"#,
        );
        files::write_string(&path, raw, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let queue = Queue::from_snapshot(8, open(&path).await, Persistence::BestEffort);

        let found = queue.next_ready(Utc::now()).unwrap().job;
        let expected = ReportMetadata {
            role: None,
            aspect: None,
            serial_number: None,
            deployment_id: None,
            ..metadata("arm")
        };
        assert_eq!(found, expected);
    }

    #[tokio::test]
    async fn enqueue_writes_the_pinned_metadata_keys() {
        let dir = test_dirs::temp("calibration_queue_test").unwrap();
        let path = dir.to_dir().file("queue.json");
        {
            let mut queue = Queue::from_snapshot(8, open(&path).await, Persistence::BestEffort);
            queue.enqueue(metadata("arm")).await.unwrap();
        }

        let raw = files::read_string(&path).await.unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let metadata = value["entries"][0]["job"].as_object().unwrap();
        let mut keys: Vec<&str> = metadata.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "aspect",
                "calibrated_at",
                "component_slug",
                "deployment_id",
                "format",
                "role",
                "serial_number",
            ],
            "{raw}"
        );
    }
}
