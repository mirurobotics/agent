// internal crates
use miru_agent::data_uploads::retention::Job;
use miru_agent::filesys::{dirs, files, Dir, File, PathExt};

// external crates
use chrono::{DateTime, Utc};

/// A zero-TTL `Job` whose recorded size and mtime match a freshly created
/// directory (`<dir>/undeletable`) represented as a `File`. Stat and the
/// identity check succeed, but unlinking a directory through the file path
/// fails on every platform, so a sweep reaches the delete step and fails
/// there. Override `mtime` to force the digest branch instead.
pub async fn undeletable_dir_job(dir: &Dir, observed_at: DateTime<Utc>) -> Job {
    let target = dir.subdir("undeletable");
    dirs::create(&target).await.unwrap();
    let file = File::new(target.path().clone());
    let metadata = files::metadata(&file).await.unwrap();
    Job {
        file,
        size: metadata.len(),
        digest: "sha256:unused".to_string(),
        mtime: DateTime::<Utc>::from(metadata.modified().unwrap()),
        first_observed_at: observed_at,
        last_observed_at: observed_at,
        ttl_secs: 0,
        file_rule_id: "rule_1".to_string(),
        deployment_id: "dpl_1".to_string(),
    }
}
