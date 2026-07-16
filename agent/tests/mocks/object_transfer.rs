// standard crates
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

// internal crates
use backend_api::models::{UploadCredentials, UploadDestination};
use miru_agent::filesys::File;
use miru_agent::upload::errors::ExecutorErr;
use miru_agent::upload::{ObjectTransfer, UploadErr};

/// Scripted [`ObjectTransfer`] test double. Each `transfer` call pops the next
/// scripted result (an empty script defaults to `Ok`) and records the call's
/// arguments. Clones share the script and the recorded calls, so a test keeps
/// one clone for assertions after moving the other into a `LiveExecutor`
/// (which owns its transfer).
#[derive(Clone, Default)]
pub struct MockObjectTransfer {
    script: Arc<Mutex<VecDeque<Result<(), UploadErr>>>>,
    calls: Arc<Mutex<Vec<(UploadCredentials, UploadDestination, File, Option<u32>)>>>,
}

impl MockObjectTransfer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_ok(&self) {
        self.script.lock().unwrap().push_back(Ok(()));
    }

    pub fn push_err(&self) {
        self.script
            .lock()
            .unwrap()
            .push_back(Err(UploadErr::ExecutorErr(ExecutorErr {
                source: Box::new(std::io::Error::other("scripted transfer failure")),
                trace: miru_agent::trace!(),
            })));
    }

    pub fn recorded_calls(&self) -> Vec<(UploadCredentials, UploadDestination, File, Option<u32>)> {
        self.calls.lock().unwrap().clone()
    }
}

impl ObjectTransfer for MockObjectTransfer {
    async fn transfer(
        &self,
        credentials: &UploadCredentials,
        destination: &UploadDestination,
        file: &File,
        expected_crc32c: Option<u32>,
    ) -> Result<(), UploadErr> {
        self.calls.lock().unwrap().push((
            credentials.clone(),
            destination.clone(),
            file.clone(),
            expected_crc32c,
        ));
        let step = self.script.lock().unwrap().pop_front();
        step.unwrap_or(Ok(()))
    }
}
