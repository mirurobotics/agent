// standard crates
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

// internal crates
use backend_api::models::{MultipartUpload, UploadWithCredentials};
use miru_agent::data_uploads::upload::errors::ExecutorErr;
use miru_agent::data_uploads::upload::{ObjectTransfer, UploadErr};
use miru_agent::filesys::File;

/// One recorded `transfer` call's arguments.
pub type TransferCall = (UploadWithCredentials, File, Option<MultipartUpload>);

/// Scripted [`ObjectTransfer`] test double. Each `transfer` call pops the next
/// scripted result (an empty script defaults to `Ok`) and records the call's
/// arguments. `multipart_upload` returns the one recorded on the upload, unless
/// [`Self::start_multipart_upload`] set a new one. Clones share the script and
/// the recorded calls, so a test keeps one clone for assertions after moving
/// the other into a `LiveExecutor` (which owns its transfer).
#[derive(Clone, Default)]
pub struct MockObjectTransfer {
    script: Arc<Mutex<VecDeque<Result<(), UploadErr>>>>,
    started: Arc<Mutex<Option<MultipartUpload>>>,
    calls: Arc<Mutex<Vec<TransferCall>>>,
}

impl MockObjectTransfer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_err(&self) {
        self.script
            .lock()
            .unwrap()
            .push_back(Err(UploadErr::ExecutorErr(ExecutorErr {
                source: Box::new(std::io::Error::other("scripted transfer failure")),
                is_terminal: false,
                is_network_conn_err: false,
                trace: miru_agent::trace!(),
            })));
    }

    /// Makes `multipart_upload` return `multipart`, as if it started a new one.
    pub fn start_multipart_upload(&self, multipart: MultipartUpload) {
        *self.started.lock().unwrap() = Some(multipart);
    }

    pub fn recorded_calls(&self) -> Vec<TransferCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl ObjectTransfer for MockObjectTransfer {
    async fn multipart_upload(
        &self,
        upload: &UploadWithCredentials,
        _size: u64,
    ) -> Result<Option<MultipartUpload>, UploadErr> {
        let started = self.started.lock().unwrap().clone();
        Ok(started.or_else(|| upload.upload.multipart_upload.as_deref().cloned()))
    }

    async fn transfer(
        &self,
        upload: &UploadWithCredentials,
        file: &File,
        multipart: Option<&MultipartUpload>,
    ) -> Result<(), UploadErr> {
        self.calls
            .lock()
            .unwrap()
            .push((upload.clone(), file.clone(), multipart.cloned()));
        let step = self.script.lock().unwrap().pop_front();
        step.unwrap_or(Ok(()))
    }
}
