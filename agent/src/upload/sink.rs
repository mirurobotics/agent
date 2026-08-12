// standard crates
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

// internal crates
use crate::models::FileRule;
use crate::scan::{scanner::StableFile, sink::StableFileSink};
use crate::upload::{Job, Uploader, UploaderExt};

// external crates
use tracing::{debug, warn};

/// The upload policy over the scanner's stable-file stream: an upload-bearing
/// rule's stable file becomes an enqueued upload [`Job`]; a retention-only
/// rule's stable file is skipped. Enqueue failures are logged and swallowed —
/// the sink is infallible from the scanner's perspective.
pub struct UploadStableFileSink {
    uploader: Arc<Uploader>,
}

impl UploadStableFileSink {
    pub fn new(uploader: Arc<Uploader>) -> Self {
        Self { uploader }
    }
}

impl StableFileSink for UploadStableFileSink {
    fn on_stable_file<'a>(
        &'a self,
        file: StableFile,
        rule: &'a FileRule,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            if rule.upload.is_none() {
                let path = &file.file;
                let rule_id = &rule.id;
                debug!("upload sink: rule {rule_id} does not upload; skipping {path}");
                return;
            }

            let job = Job {
                file: file.file,
                size: file.size,
                digest: file.digest,
                mtime: file.mtime,
                first_observed_at: file.first_observed_at,
                last_observed_at: file.last_observed_at,
                file_rule_id: file.file_rule_id,
                deployment_id: file.deployment_id,
                retention: file.retention,
            };

            if let Err(e) = self.uploader.enqueue(job).await {
                warn!("upload sink: failed to enqueue upload job: {e:?}");
            }
        })
    }
}
