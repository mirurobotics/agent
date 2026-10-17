// internal crates
use crate::mocks::deleter::{MockDeleter, MockStep};
use miru_agent::data_uploads::retention::{Job, RetentionStableFileSink};
use miru_agent::data_uploads::scan::{scanner::StableFile, StableFileSink};
use miru_agent::filesys::File;
use miru_agent::models::{FileRule, FileRuleRetention, FileRuleUpload};

// external crates
use chrono::DateTime;

fn stable_file(name: &str) -> StableFile {
    StableFile {
        file: File::new(format!("/data/{name}")),
        size: 128,
        digest: format!("sha256:{name}"),
        mtime: DateTime::from_timestamp(950, 0).unwrap(),
        mtime_aliases: Vec::new(),
        first_observed_at: DateTime::from_timestamp(900, 0).unwrap(),
        last_observed_at: DateTime::from_timestamp(1000, 0).unwrap(),
        deployment_id: "dpl_1".to_string(),
        file_rule_id: "rule_1".to_string(),
    }
}

fn rule(upload: Option<FileRuleUpload>, retention: Option<FileRuleRetention>) -> FileRule {
    FileRule {
        upload,
        retention,
        ..FileRule::default()
    }
}

/// The delete job the sink is expected to build from [`stable_file`]'s
/// observation and the rule's `ttl_secs`.
fn expected_job(name: &str, ttl_secs: u64) -> Job {
    Job {
        file: File::new(format!("/data/{name}")),
        size: 128,
        digest: format!("sha256:{name}"),
        mtime: DateTime::from_timestamp(950, 0).unwrap(),
        first_observed_at: DateTime::from_timestamp(900, 0).unwrap(),
        last_observed_at: DateTime::from_timestamp(1000, 0).unwrap(),
        ttl_secs,
        file_rule_id: "rule_1".to_string(),
        deployment_id: "dpl_1".to_string(),
    }
}

#[tokio::test]
async fn upload_with_unrequired_retention_enqueues_a_delete_job() {
    let deleter = MockDeleter::new();
    let sink = RetentionStableFileSink::new(deleter.clone());
    let rule = rule(
        Some(FileRuleUpload::default()),
        Some(FileRuleRetention {
            require_upload: Some(false),
            ttl_secs: 3600,
        }),
    );

    sink.on_stable_file(stable_file("a.log"), &rule).await;

    assert_eq!(deleter.recorded_calls(), [expected_job("a.log", 3600)]);
}

// The gate is retention-not-requiring-upload, independent of the upload block:
// a retention-only rule's files are deletable at stability too.
#[tokio::test]
async fn retention_only_rule_enqueues_a_delete_job() {
    let deleter = MockDeleter::new();
    let sink = RetentionStableFileSink::new(deleter.clone());
    let rule = rule(
        None,
        Some(FileRuleRetention {
            require_upload: Some(false),
            ttl_secs: 0,
        }),
    );

    sink.on_stable_file(stable_file("b.log"), &rule).await;

    assert_eq!(deleter.recorded_calls(), [expected_job("b.log", 0)]);
}

// The backend omits require_upload when the rule has no upload block. On a
// rule without uploads, that absence is the same gate as false: the file is
// deletable at stability.
#[tokio::test]
async fn absent_require_upload_enqueues_a_delete_job() {
    let deleter = MockDeleter::new();
    let sink = RetentionStableFileSink::new(deleter.clone());
    let rule = rule(
        None,
        Some(FileRuleRetention {
            require_upload: None,
            ttl_secs: 60,
        }),
    );

    sink.on_stable_file(stable_file("f.log"), &rule).await;

    assert_eq!(deleter.recorded_calls(), [expected_job("f.log", 60)]);
}

// A rule that uploads but arrives without require_upload (e.g. a stale cached
// rule) must not delete at stability: the file would be gone before it was
// uploaded. The absence defaults to requiring the upload.
#[tokio::test]
async fn upload_rule_with_absent_require_upload_enqueues_nothing() {
    let deleter = MockDeleter::new();
    let sink = RetentionStableFileSink::new(deleter.clone());
    let rule = rule(
        Some(FileRuleUpload::default()),
        Some(FileRuleRetention {
            require_upload: None,
            ttl_secs: 60,
        }),
    );

    sink.on_stable_file(stable_file("g.log"), &rule).await;

    assert_eq!(deleter.recorded_calls(), []);
}

// require_upload files become deletable at upload confirmation, not at
// stability — enqueueing here too would double-enqueue.
#[tokio::test]
async fn required_upload_retention_enqueues_nothing() {
    let deleter = MockDeleter::new();
    let sink = RetentionStableFileSink::new(deleter.clone());
    let rule = rule(
        Some(FileRuleUpload::default()),
        Some(FileRuleRetention {
            require_upload: Some(true),
            ttl_secs: 0,
        }),
    );

    sink.on_stable_file(stable_file("c.log"), &rule).await;

    assert_eq!(deleter.recorded_calls(), []);
}

#[tokio::test]
async fn absent_retention_enqueues_nothing() {
    let deleter = MockDeleter::new();
    let sink = RetentionStableFileSink::new(deleter.clone());
    let rule = rule(Some(FileRuleUpload::default()), None);

    sink.on_stable_file(stable_file("d.log"), &rule).await;

    assert_eq!(deleter.recorded_calls(), []);
}

// The sink is infallible from the scanner's perspective: an enqueue error is
// logged and swallowed, never propagated into the scan tick.
#[tokio::test]
async fn enqueue_failure_is_swallowed() {
    let deleter = MockDeleter::new();
    deleter.push_step(MockStep::Err);
    let sink = RetentionStableFileSink::new(deleter.clone());
    let rule = rule(
        None,
        Some(FileRuleRetention {
            require_upload: Some(false),
            ttl_secs: 60,
        }),
    );

    sink.on_stable_file(stable_file("e.log"), &rule).await;

    // the failed enqueue was attempted and the sink completed without panic.
    assert_eq!(deleter.recorded_calls(), [expected_job("e.log", 60)]);
}
