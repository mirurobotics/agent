// standard crates
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

// internal crates
use miru_agent::delete::errors::QueueFullErr;
use miru_agent::delete::{DeleteErr, DeleterExt, PendingDelete};

/// One scripted result for a `MockDeleter::enqueue` call.
pub enum MockStep {
    Ok,
    Err,
}

/// A test double for [`DeleterExt`] that records every `enqueue`d
/// [`PendingDelete`] and follows a scripted result queue (an empty script
/// defaults to `Ok`), mirroring `MockUploadExecutor`. The other trait methods
/// return sensible defaults.
pub struct MockDeleter {
    script: Mutex<VecDeque<MockStep>>,
    pub calls: Mutex<Vec<PendingDelete>>,
}

impl MockDeleter {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            script: Mutex::new(VecDeque::new()),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub fn push_step(&self, step: MockStep) {
        self.script.lock().unwrap().push_back(step);
    }

    pub fn recorded_calls(&self) -> Vec<PendingDelete> {
        self.calls.lock().unwrap().clone()
    }
}

impl DeleterExt for MockDeleter {
    async fn enqueue(&self, pending: PendingDelete) -> Result<(), DeleteErr> {
        let file = pending.file.to_string();
        self.calls.lock().unwrap().push(pending);
        let step = self.script.lock().unwrap().pop_front();
        match step {
            None | Some(MockStep::Ok) => Ok(()),
            Some(MockStep::Err) => Err(DeleteErr::QueueFullErr(QueueFullErr {
                capacity: 0,
                file,
                trace: miru_agent::trace!(),
            })),
        }
    }

    async fn sweep(&self) -> Result<(), DeleteErr> {
        Ok(())
    }

    async fn len(&self) -> Result<usize, DeleteErr> {
        Ok(self.calls.lock().unwrap().len())
    }

    async fn shutdown(&self) -> Result<(), DeleteErr> {
        Ok(())
    }
}
