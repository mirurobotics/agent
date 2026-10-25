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

/// Scripted [`ObjectTransfer`] test double. `prepare_multipart` returns the
/// multipart upload given to [`Self::set_multipart`], or else the one on the
/// upload, and fails after [`Self::fail_multipart`]. Each `transfer` call pops
/// the next scripted result (an empty script defaults to `Ok`) and records the
/// call's arguments. Clones share the script and the recorded calls, so a test
/// keeps one clone for assertions after moving the other into a `LiveExecutor`
/// (which owns its transfer).
#[derive(Clone, Default)]
pub struct MockObjectTransfer {
    multipart: Arc<Mutex<Option<MultipartCheckpoint>>>,
    fail_multipart: Arc<Mutex<bool>>,
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

    pub fn fail_multipart(&self) {
        *self.fail_multipart.lock().unwrap() = true;
    }

    pub fn push_err(&self) {
        self.script.lock().unwrap().push_back(Err(scripted_err()));
    }

    pub fn recorded_calls(&self) -> Vec<TransferCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl ObjectTransfer for MockObjectTransfer {
    async fn prepare_multipart(
        &self,
        _credentials: &UploadCredentials,
        upload: &UploadCheckpoint,
        _file: &File,
    ) -> Result<Option<MultipartCheckpoint>, UploadErr> {
        if *self.fail_multipart.lock().unwrap() {
            return Err(scripted_err());
        }
        let started = self.multipart.lock().unwrap().clone();
        Ok(started.or_else(|| upload.multipart.clone()))
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

fn scripted_err() -> UploadErr {
    UploadErr::ExecutorErr(ExecutorErr {
        source: Box::new(std::io::Error::other("scripted transfer failure")),
        is_terminal: false,
        is_network_conn_err: false,
        trace: miru_agent::trace!(),
    })
}
