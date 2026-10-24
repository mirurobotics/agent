// standard crates
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

// internal crates
use backend_api::models::{UploadCredentials, UploadDestination};
use miru_agent::data_uploads::upload::errors::ExecutorErr;
use miru_agent::data_uploads::upload::{
    Checkpoint, Checkpointer, ObjectTransfer, TransferState, UploadErr,
};
use miru_agent::filesys::File;

/// One recorded `transfer` call's arguments.
pub type TransferCall = (
    UploadCredentials,
    UploadDestination,
    File,
    HashMap<String, String>,
);

/// Scripted [`ObjectTransfer`] test double. Each `transfer` call pops the next
/// scripted result (an empty script defaults to `Ok`) and records the call's
/// arguments. Clones share the script and the recorded calls, so a test keeps
/// one clone for assertions after moving the other into a `LiveExecutor`
/// (which owns its transfer).
#[derive(Clone, Default)]
pub struct MockObjectTransfer {
    script: Arc<Mutex<VecDeque<Result<(), UploadErr>>>>,
    calls: Arc<Mutex<Vec<TransferCall>>>,
    states: Arc<Mutex<Vec<Option<TransferState>>>>,
    checkpoints: Arc<Mutex<VecDeque<Vec<Checkpoint>>>>,
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

    /// Script the checkpoints the next `transfer` call saves before it
    /// returns its scripted result.
    pub fn push_checkpoints(&self, checkpoints: Vec<Checkpoint>) {
        self.checkpoints.lock().unwrap().push_back(checkpoints);
    }

    pub fn recorded_calls(&self) -> Vec<TransferCall> {
        self.calls.lock().unwrap().clone()
    }

    /// The `state` each `transfer` call received, in call order.
    pub fn recorded_states(&self) -> Vec<Option<TransferState>> {
        self.states.lock().unwrap().clone()
    }
}

impl ObjectTransfer for MockObjectTransfer {
    async fn transfer<K: Checkpointer>(
        &self,
        credentials: &UploadCredentials,
        destination: &UploadDestination,
        file: &File,
        metadata: &HashMap<String, String>,
        state: Option<&TransferState>,
        checkpointer: &K,
    ) -> Result<(), UploadErr> {
        self.calls.lock().unwrap().push((
            credentials.clone(),
            destination.clone(),
            file.clone(),
            metadata.clone(),
        ));
        self.states.lock().unwrap().push(state.cloned());
        let checkpoints = self.checkpoints.lock().unwrap().pop_front();
        for checkpoint in checkpoints.unwrap_or_default() {
            checkpointer.save(checkpoint).await;
        }
        let step = self.script.lock().unwrap().pop_front();
        step.unwrap_or(Ok(()))
    }
}
