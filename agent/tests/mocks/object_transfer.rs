// standard crates
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

// internal crates
use backend_api::models::UploadCredentials;
use miru_agent::data_uploads::upload::errors::ExecutorErr;
use miru_agent::data_uploads::upload::{
    MultipartCheckpoint, ObjectTransfer, UploadCheckpoint, UploadErr,
};
use miru_agent::filesys::File;

/// One recorded `transfer` call's arguments.
pub type TransferCall = (UploadCredentials, UploadCheckpoint, File);

/// Scripted [`ObjectTransfer`] test double. `prepare` returns the upload with
/// the multipart upload given to [`Self::set_multipart`], if any. Each
/// `transfer` call pops the next scripted result (an empty script defaults to
/// `Ok`) and records the call's arguments. Clones share the script and the
/// recorded calls, so a test keeps one clone for assertions after moving the
/// other into a `LiveExecutor` (which owns its transfer).
#[derive(Clone, Default)]
pub struct MockObjectTransfer {
    multipart: Arc<Mutex<Option<MultipartCheckpoint>>>,
    script: Arc<Mutex<VecDeque<Result<(), UploadErr>>>>,
    calls: Arc<Mutex<Vec<TransferCall>>>,
}

impl MockObjectTransfer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_multipart(&self, multipart: MultipartCheckpoint) {
        *self.multipart.lock().unwrap() = Some(multipart);
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

    pub fn recorded_calls(&self) -> Vec<TransferCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl ObjectTransfer for MockObjectTransfer {
    async fn prepare(
        &self,
        _credentials: &UploadCredentials,
        mut upload: UploadCheckpoint,
        _file: &File,
    ) -> UploadCheckpoint {
        if let Some(multipart) = self.multipart.lock().unwrap().clone() {
            upload.multipart = Some(multipart);
        }
        upload
    }

    async fn transfer(
        &self,
        credentials: &UploadCredentials,
        upload: &UploadCheckpoint,
        file: &File,
    ) -> Result<(), UploadErr> {
        self.calls
            .lock()
            .unwrap()
            .push((credentials.clone(), upload.clone(), file.clone()));
        let step = self.script.lock().unwrap().pop_front();
        step.unwrap_or(Ok(()))
    }
}
