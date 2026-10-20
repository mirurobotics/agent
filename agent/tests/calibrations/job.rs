//! The calibration queue: the generic suite instantiated for `CalibrationJob`,
//! plus the on-disk wire pin for `calibrations/queue.json`.

// internal crates
use crate::queue::queue_suite;
use crate::test_utils::filesys::dirs as test_dirs;
use miru_agent::calibrations::{CalibrationJob, Queue, QueueSnapshot, QueueSnapshotFile};
use miru_agent::queue::QueueJob;
use miru_agent::filesys::{files, File, WriteOptions};

// external crates
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// A deterministic calibration job, due the moment it arrives.
fn calibration_job(name: &str) -> CalibrationJob {
    CalibrationJob {
        component_slug: name.to_string(),
        role: Some("aligner-1".to_string()),
        aspect: Some("pose".to_string()),
        serial_number: Some("SN-1".to_string()),
        deployment_id: Some("dpl_1".to_string()),
        calibrated_at: DateTime::from_timestamp(1000, 0).unwrap(),
    }
}

queue_suite!(calibration_job, "calibration-generic-queue-test");

#[test]
fn due_at_is_min_utc() {
    assert_eq!(
        QueueJob::due_at(&calibration_job("arm")),
        DateTime::<Utc>::MIN_UTC
    );
}

#[test]
fn name_is_the_component_and_time() {
    assert_eq!(
        QueueJob::name(&calibration_job("arm")),
        "calibration of arm received at 1970-01-01T00:16:40+00:00"
    );
}

async fn open(path: &File) -> QueueSnapshotFile {
    QueueSnapshotFile::new_with_default(path.clone(), QueueSnapshot::default())
        .await
        .unwrap()
}

mod wire {
    use super::*;

    /// Pins the persisted shape of `queue.json`. It holds buffered reports the
    /// agent has promised to deliver, so a shape change must fail here rather
    /// than strand a live buffer.
    #[tokio::test]
    async fn raw_json_snapshot_loads() {
        let dir = test_dirs::temp("calibration_queue_test").unwrap();
        let path = dir.to_dir().file("queue.json");
        let id = Uuid::new_v4();
        let raw = format!(
            concat!(
                r#"{{"entries":[{{"id":"{id}","job":{{"#,
                r#""component_slug":"arm","#,
                r#""role":"aligner-1","aspect":"pose","serial_number":"SN-1","#,
                r#""deployment_id":"dpl_1","calibrated_at":"1970-01-01T00:16:40Z""#,
                r#"}},"attempts":0,"next_attempt_at":null}}]}}"#,
            ),
            id = id,
        );
        files::write_string(&path, &raw, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let queue = Queue::from_snapshot(8, open(&path).await);

        let entry = queue.next_ready(Utc::now()).unwrap();
        assert_eq!(entry.id, id);
        assert_eq!(entry.job, calibration_job("arm"));
    }

    /// The optional fields may be absent, as for a calibration taken with no
    /// deployment, role, aspect or serial number.
    #[tokio::test]
    async fn optional_fields_default_to_none() {
        let dir = test_dirs::temp("calibration_queue_test").unwrap();
        let path = dir.to_dir().file("queue.json");
        let raw = concat!(
            r#"{"entries":[{"job":{"component_slug":"arm","#,
            r#""calibrated_at":"1970-01-01T00:16:40Z"}}]}"#,
        );
        files::write_string(&path, raw, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let queue = Queue::from_snapshot(8, open(&path).await);

        let job = queue.next_ready(Utc::now()).unwrap().job;
        let expected = CalibrationJob {
            role: None,
            aspect: None,
            serial_number: None,
            deployment_id: None,
            ..calibration_job("arm")
        };
        assert_eq!(job, expected);
    }

    #[tokio::test]
    async fn enqueue_writes_the_pinned_job_keys() {
        let dir = test_dirs::temp("calibration_queue_test").unwrap();
        let path = dir.to_dir().file("queue.json");
        {
            let mut queue = Queue::from_snapshot(8, open(&path).await);
            queue.enqueue(calibration_job("arm")).await.unwrap();
        }

        let raw = files::read_string(&path).await.unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let job = value["entries"][0]["job"].as_object().unwrap();
        let mut keys: Vec<&str> = job.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "aspect",
                "calibrated_at",
                "component_slug",
                "deployment_id",
                "role",
                "serial_number",
            ],
            "{raw}"
        );
    }
}
