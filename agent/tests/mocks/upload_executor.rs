// standard crates
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

// internal crates
use backend_api::models::UploadCredentials;
use miru_agent::data_uploads::upload::errors::ExecutorErr;
use miru_agent::data_uploads::upload::{
    Job, PreparedUpload, UploadCheckpoint, UploadErr, UploadExecutor,
};

// external crates
use tokio::sync::{mpsc, oneshot};

/// One scripted result for an attempt on a `MockUploadExecutor`.
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
    /// Return this checkpoint from `prepare` for the actor to save, then run
    /// the inner step in `upload`.
    PrepareThen(UploadCheckpoint, Box<MockStep>),
}

pub struct MockUploadExecutor {
    script: Mutex<VecDeque<MockStep>>,
    pub calls: Mutex<Vec<Job>>,
    started_tx: mpsc::UnboundedSender<()>,
}

impl MockUploadExecutor {
    /// Returns the mock plus the receiver notified at the start of every
    /// `prepare` and `upload` call (after the call is recorded, before the
    /// result is produced), for deterministic sequencing in tests.
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

    /// The job of every attempt, recorded by `prepare`.
    pub fn recorded_calls(&self) -> Vec<Job> {
        self.calls.lock().unwrap().clone()
    }

    /// Pops the next step and notifies the started receiver.
    fn next_step(&self) -> Option<MockStep> {
        let step = self.script.lock().unwrap().pop_front();
        // the mutex guard above is already dropped: never hold a
        // std::sync::Mutex guard across an await or the future is not Send
        let _ = self.started_tx.send(());
        step
    }
}

impl UploadExecutor for MockUploadExecutor {
    /// Runs the attempt's step. Any step but [`MockStep::PrepareThen`] ends
    /// the attempt here, as an already confirmed upload does.
    async fn prepare(&self, job: &Job) -> Result<Option<PreparedUpload>, UploadErr> {
        self.calls.lock().unwrap().push(job.clone());
        match self.next_step() {
            Some(MockStep::PrepareThen(checkpoint, then)) => {
                self.script.lock().unwrap().push_front(*then);
                Ok(Some(PreparedUpload {
                    checkpoint,
                    credentials: UploadCredentials::default(),
                }))
            }
            step => play(step).await.map(|()| None),
        }
    }

    async fn upload(&self, _job: &Job, _prepared: &PreparedUpload) -> Result<(), UploadErr> {
        play(self.next_step()).await
    }
}

async fn play(step: Option<MockStep>) -> Result<(), UploadErr> {
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
        Some(MockStep::PrepareThen(..)) => panic!("PrepareThen can only start an attempt"),
    }
}
