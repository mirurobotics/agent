// standard crates
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

// internal crates
use miru_agent::data_uploads::upload::errors::ExecutorErr;
use miru_agent::data_uploads::upload::{Job, UploadErr, UploadExecutor};

// external crates
use tokio::sync::{mpsc, oneshot};

/// One scripted result for a `MockUploadExecutor::upload` call.
pub enum MockStep {
    Ok,
    Err,
    /// Fail with an error classified terminal (HTTP 400)
    TerminalErr,
    /// Fail with an error classified as a network connection error
    NetworkErr,
    /// Await the receiver and return the sent result (or `Ok(())` if the
    /// sender was dropped). The test holds the sender, so it controls when —
    /// and how — the in-flight upload finishes.
    Hang(oneshot::Receiver<Result<(), UploadErr>>),
}

pub struct MockUploadExecutor {
    script: Mutex<VecDeque<MockStep>>,
    pub calls: Mutex<Vec<Job>>,
    started_tx: mpsc::UnboundedSender<()>,
}

impl MockUploadExecutor {
    /// Returns the mock plus the receiver notified at the start of every
    /// `upload` call (after the call is recorded, before the result is
    /// produced), for deterministic sequencing in tests.
    pub fn new() -> (Arc<Self>, mpsc::UnboundedReceiver<()>) {
        let (started_tx, started_rx) = mpsc::unbounded_channel();
        (
            Arc::new(Self {
                script: Mutex::new(VecDeque::new()),
                calls: Mutex::new(Vec::new()),
                started_tx,
            }),
            started_rx,
        )
    }

    pub fn push_step(&self, step: MockStep) {
        self.script.lock().unwrap().push_back(step);
    }

    pub fn recorded_calls(&self) -> Vec<Job> {
        self.calls.lock().unwrap().clone()
    }
}

impl UploadExecutor for MockUploadExecutor {
    async fn upload(&self, job: &Job) -> Result<(), UploadErr> {
        self.calls.lock().unwrap().push(job.clone());
        let step = self.script.lock().unwrap().pop_front();
        // both mutex guards above are already dropped: never hold a
        // std::sync::Mutex guard across an await or the future is not Send
        let _ = self.started_tx.send(());
        match step {
            None | Some(MockStep::Ok) => Ok(()),
            Some(MockStep::Err) => Err(UploadErr::ExecutorErr(ExecutorErr {
                source: Box::new(std::io::Error::other("scripted failure")),
                is_terminal: false,
                is_network_conn_err: false,
                trace: miru_agent::trace!(),
            })),
            Some(MockStep::TerminalErr) => Err(UploadErr::ExecutorErr(ExecutorErr {
                source: Box::new(std::io::Error::other("scripted terminal failure")),
                is_terminal: true,
                is_network_conn_err: false,
                trace: miru_agent::trace!(),
            })),
            Some(MockStep::NetworkErr) => Err(UploadErr::ExecutorErr(ExecutorErr {
                source: Box::new(std::io::Error::other("scripted network failure")),
                is_terminal: false,
                is_network_conn_err: true,
                trace: miru_agent::trace!(),
            })),
            Some(MockStep::Hang(rx)) => rx.await.unwrap_or(Ok(())),
        }
    }
}
