//! The calibration queue: the generic suite instantiated for `Job`,
//! plus the on-disk wire pin for `calibrations/queue.json`.

// internal crates
use crate::queue::file::queue_suite;
use crate::test_utils::calibrations::{full_metadata, received_at, report_id};
use crate::test_utils::filesys::dirs as test_dirs;
use miru_daemon::calibrations::{
    Invalidation, Job, Queue, QueueSnapshot, QueueSnapshotFile, ReportMetadata,
};
use miru_daemon::filesys::{files, File, WriteOptions};
use miru_daemon::queue::{Persistence, QueueJob};

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use uuid::Uuid;

/// The upload of `full_metadata(name)`.
fn report_job(name: &str) -> Job {
    Job::Report(full_metadata(name))
}

/// The upload of the invalidation of `full_metadata(name)`.
fn invalidation_job(name: &str) -> Job {
    Job::Invalidation(Invalidation {
        report_id: report_id(name),
        invalidated_at: received_at() + TimeDelta::hours(1),
        reason: "sensor was bumped".to_string(),
    })
}

queue_suite!(report_job, "calibration-generic-queue-test");

#[test]
fn ready_at_is_min_utc() {
    assert_eq!(
        QueueJob::ready_at(&report_job("arm")),
        DateTime::<Utc>::MIN_UTC
    );
}

#[test]
fn name_is_the_component_role_aspect_and_time() {
    assert_eq!(
        "calibration of arm (role aligner-1) (aspect pose) received at 1970-01-01T00:16:40+00:00",
        QueueJob::name(&report_job("arm"))
    );
}

#[test]
fn name_leaves_out_a_missing_role_and_aspect() {
    let report = ReportMetadata {
        role: None,
        aspect: None,
        ..full_metadata("arm")
    };

    assert_eq!(
        "calibration of arm received at 1970-01-01T00:16:40+00:00",
        QueueJob::name(&Job::Report(report))
    );
}

#[test]
fn name_of_an_invalidation_is_its_report_id_and_time() {
    let want = format!(
        "invalidation of calibration {}, made at 1970-01-01T01:16:40+00:00",
        report_id("arm")
    );
    assert_eq!(want, QueueJob::name(&invalidation_job("arm")));
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
                r#""reported_by":"Ada Lovelace","#,
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
        assert_eq!(entry.job, report_job("arm"));
    }

    /// A queued invalidation holds only its report's id, when and why.
    #[tokio::test]
    async fn an_invalidation_loads() {
        let dir = test_dirs::temp("calibration_queue_test").unwrap();
        let path = dir.to_dir().file("queue.json");
        let raw = format!(
            concat!(
                r#"{{"entries":[{{"job":{{"report_id":"{report_id}","#,
                r#""invalidated_at":"1970-01-01T01:16:40Z","#,
                r#""reason":"sensor was bumped"}}}}]}}"#,
            ),
            report_id = report_id("arm"),
        );
        files::write_string(&path, &raw, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let queue = Queue::from_snapshot(8, open(&path).await, Persistence::BestEffort);

        let found = queue.next_ready(Utc::now()).unwrap().job;
        assert_eq!(found, invalidation_job("arm"));
    }

    /// The optional fields may be absent, as for a calibration taken with no
    /// deployment, role, aspect, serial number or reporting user.
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
            reported_by: None,
            deployment_id: None,
            ..full_metadata("arm")
        };
        assert_eq!(found, Job::Report(expected));
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
            r#""reported_by":"Ada Lovelace","#,
            r#""deployment_id":"dpl_1","reported_at":"1970-01-01T00:16:40Z"}}]}"#,
        );
        files::write_string(&path, raw, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let queue = Queue::from_snapshot(8, open(&path).await, Persistence::BestEffort);

        let Job::Report(found) = queue.next_ready(Utc::now()).unwrap().job else {
            panic!("expected a report");
        };
        assert!(!found.report_id.is_nil());
        let expected = ReportMetadata {
            report_id: found.report_id,
            ..full_metadata("arm")
        };
        assert_eq!(found, expected);
    }
}
