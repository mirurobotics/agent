// standard crates
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

// internal crates
use backend_api::models::{UploadCredentials, UploadDestination};
use miru_agent::clock::Clock;
use miru_agent::data_uploads::upload::errors::ExecutorErr;
use miru_agent::data_uploads::upload::{ObjectTransfer, UploadErr};
use miru_agent::filesys::File;

// external crates
use chrono::{DateTime, Utc};

/// One recorded `transfer` call's arguments.
pub type TransferCall = (
    UploadCredentials,
    UploadDestination,
    File,
    HashMap<String, String>,
);

/// The clock a transfer moves, and the backend time it runs until on it.
type RunsUntil = Option<(Clock, DateTime<Utc>)>;

/// Scripted [`ObjectTransfer`] test double. Each `transfer` call pops the next
/// scripted result (an empty script defaults to `Ok`) and records the call's
/// arguments. Clones share the script and the recorded calls, so a test keeps
/// one clone for assertions after moving the other into a `LiveExecutor`
/// (which owns its transfer).
#[derive(Clone, Default)]
pub struct MockObjectTransfer {
    script: Arc<Mutex<VecDeque<Result<(), UploadErr>>>>,
    calls: Arc<Mutex<Vec<TransferCall>>>,
    runs_until: Arc<Mutex<RunsUntil>>,
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

    /// Makes each transfer run until `clock` reads the backend's time as
    /// `server_time`.
    pub fn run_until(&self, clock: Clock, server_time: DateTime<Utc>) {
        *self.runs_until.lock().unwrap() = Some((clock, server_time));
    }

    pub fn recorded_calls(&self) -> Vec<TransferCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl ObjectTransfer for MockObjectTransfer {
    async fn transfer(
        &self,
        credentials: &UploadCredentials,
        destination: &UploadDestination,
        file: &File,
        metadata: &HashMap<String, String>,
    ) -> Result<(), UploadErr> {
        self.calls.lock().unwrap().push((
            credentials.clone(),
            destination.clone(),
            file.clone(),
            metadata.clone(),
        ));
        let runs_until = self.runs_until.lock().unwrap().clone();
        if let Some((clock, server_time)) = runs_until {
            clock.observe_server_time(server_time);
        }
        let step = self.script.lock().unwrap().pop_front();
        step.unwrap_or(Ok(()))
    }
}
