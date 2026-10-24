// standard crates
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

// internal crates
use backend_api::models::UploadCredentials;
use miru_agent::data_uploads::upload::errors::ExecutorErr;
use miru_agent::data_uploads::upload::{Checkpointer, ObjectTransfer, UploadErr, UploadRef};
use miru_agent::filesys::File;

/// One recorded `transfer` call's arguments.
pub type TransferCall = (UploadCredentials, UploadRef, File);

/// Scripted [`ObjectTransfer`] test double. Each `transfer` call saves the next
/// scripted upload, if any, pops the next scripted result (an empty script
/// defaults to `Ok`) and records the call's arguments. Clones share the scripts
/// and the recorded calls, so a test keeps one clone for assertions after
/// moving the other into a `LiveExecutor` (which owns its transfer).
#[derive(Clone, Default)]
pub struct MockObjectTransfer {
    script: Arc<Mutex<VecDeque<Result<(), UploadErr>>>>,
    saves: Arc<Mutex<VecDeque<UploadRef>>>,
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

    /// Script an upload for the next `transfer` call to save, as a transfer
    /// that starts a multipart upload does.
    pub fn push_save(&self, upload: UploadRef) {
        self.saves.lock().unwrap().push_back(upload);
    }

    pub fn recorded_calls(&self) -> Vec<TransferCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl ObjectTransfer for MockObjectTransfer {
    async fn transfer<K: Checkpointer>(
        &self,
        credentials: &UploadCredentials,
        upload: &UploadRef,
        file: &File,
        checkpointer: &K,
    ) -> Result<(), UploadErr> {
        self.calls
            .lock()
            .unwrap()
            .push((credentials.clone(), upload.clone(), file.clone()));
        let save = self.saves.lock().unwrap().pop_front();
        if let Some(save) = save {
            checkpointer.save(&save).await;
        }
        let step = self.script.lock().unwrap().pop_front();
        step.unwrap_or(Ok(()))
    }
}
