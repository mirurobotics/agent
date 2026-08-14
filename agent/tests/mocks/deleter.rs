// standard crates
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

// internal crates
use miru_agent::data_uploads::retention::errors::QueueFullErr;
use miru_agent::data_uploads::retention::{DeleteErr, DeleterExt, Job};

type ResultFn = Box<dyn Fn() -> Result<(), DeleteErr> + Send + Sync>;

/// One scripted result for a `MockDeleter::enqueue` call.
pub enum MockStep {
    Ok,
    Err,
}

/// A test double for [`DeleterExt`] that records every `enqueue`d
/// [`Job`] and follows a scripted result queue (an empty script
/// defaults to `Ok`). `sweep` calls are counted with a settable
/// result. The other trait methods return sensible defaults.
pub struct MockDeleter {
    script: Mutex<VecDeque<MockStep>>,
    pub calls: Mutex<Vec<Job>>,
    num_sweep_calls: AtomicUsize,
    sweep_fn: Arc<Mutex<ResultFn>>,
}

impl MockDeleter {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            script: Mutex::new(VecDeque::new()),
            calls: Mutex::new(Vec::new()),
            num_sweep_calls: AtomicUsize::new(0),
            sweep_fn: Arc::new(Mutex::new(Box::new(|| Ok(())))),
        })
    }

    pub fn push_step(&self, step: MockStep) {
        self.script.lock().unwrap().push_back(step);
    }

    pub fn recorded_calls(&self) -> Vec<Job> {
        self.calls.lock().unwrap().clone()
    }

    /// The number of `sweep` calls.
    pub fn num_sweep_calls(&self) -> usize {
        self.num_sweep_calls.load(Ordering::Relaxed)
    }

    /// Override the result returned by `sweep` (the call is still counted
    /// before the result is produced).
    pub fn set_sweep<F>(&self, f: F)
    where
        F: Fn() -> Result<(), DeleteErr> + Send + Sync + 'static,
    {
        *self.sweep_fn.lock().unwrap() = Box::new(f);
    }
}

impl DeleterExt for MockDeleter {
    async fn enqueue(&self, job: Job) -> Result<(), DeleteErr> {
        let file = job.file.to_string();
        self.calls.lock().unwrap().push(job);
        let step = self.script.lock().unwrap().pop_front();
        match step {
            None | Some(MockStep::Ok) => Ok(()),
            Some(MockStep::Err) => Err(QueueFullErr::new("delete", 0, file).into()),
        }
    }

    async fn sweep(&self) -> Result<(), DeleteErr> {
        self.num_sweep_calls.fetch_add(1, Ordering::Relaxed);
        (*self.sweep_fn.lock().unwrap())()
    }

    async fn len(&self) -> Result<usize, DeleteErr> {
        Ok(self.calls.lock().unwrap().len())
    }

    async fn shutdown(&self) -> Result<(), DeleteErr> {
        Ok(())
    }
}
