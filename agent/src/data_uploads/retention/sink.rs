// standard crates
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

// internal crates
use crate::data_uploads::{
    retention::{DeleterExt, Job},
    scan::{scanner::StableFile, sink::StableFileSink},
};
use crate::models::FileRule;

// external crates
use tracing::{debug, warn};

/// The stability-eligibility policy over the scanner's stable-file stream: a
/// rule whose retention does not require an upload has its stable files
/// enqueued for deletion `ttl_secs` after their last observation. Files whose
/// retention requires an upload become eligible at upload confirmation
/// instead, and files without retention are never deleted. Enqueue failures
/// are logged and swallowed — the sink is infallible from the scanner's
/// perspective.
pub struct RetentionStableFileSink<D: DeleterExt> {
    deleter: Arc<D>,
}

impl<D: DeleterExt> RetentionStableFileSink<D> {
    pub fn new(deleter: Arc<D>) -> Self {
        Self { deleter }
    }
}

impl<D: DeleterExt> StableFileSink for RetentionStableFileSink<D> {
    fn on_stable_file<'a>(
        &'a self,
        file: StableFile,
        rule: &'a FileRule,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let Some(retention) = &rule.retention else {
                return;
            };
            if retention.require_upload {
                let path = &file.file;
                let rule_id = &rule.id;
                debug!(
                    "retention sink: rule {rule_id} requires upload before deletion; skipping {path}"
                );
                return;
            }

            let job = Job {
                file: file.file,
                size: file.size,
                digest: file.digest,
                mtime: file.mtime,
                first_observed_at: file.first_observed_at,
                last_observed_at: file.last_observed_at,
                ttl_secs: retention.ttl_secs,
                file_rule_id: file.file_rule_id,
                deployment_id: file.deployment_id,
            };

            if let Err(e) = self.deleter.enqueue(job).await {
                warn!("retention sink: failed to enqueue delete job: {e:?}");
            }
        })
    }
}
