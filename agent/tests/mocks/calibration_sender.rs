// standard crates
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

// internal crates
use miru_agent::calibrations::{CalibrationsErr, Report, Sender};
use miru_agent::http::errors::{HTTPErr, MockErr, RequestFailed};
use miru_agent::http::request::Params;

// external crates
use tokio::sync::{mpsc, oneshot};

/// One scripted result for a `MockCalibrationSender::send` call.
pub enum Step {
    Ok,
    /// Miru rejects the report for good (HTTP 422).
    Terminal,
    /// Miru fails (HTTP 500).
    Server,
    /// Miru can't be reached.
    Network,
    /// Wait for the receiver (or its sender to drop), then succeed. The test
    /// holds the sender, so it controls when the in-flight send finishes.
    Hang(oneshot::Receiver<()>),
}

/// A scripted [`Sender`]. An empty script succeeds.
pub struct MockCalibrationSender {
    script: Mutex<VecDeque<Step>>,
    sent: Mutex<Vec<Report>>,
    started_tx: mpsc::UnboundedSender<()>,
}

impl MockCalibrationSender {
    /// The mock and a receiver signalled at the start of every `send`, after
    /// the call is recorded.
    pub fn new() -> (Arc<Self>, mpsc::UnboundedReceiver<()>) {
        let (started_tx, started_rx) = mpsc::unbounded_channel();
        let mock = Self {
            script: Mutex::new(VecDeque::new()),
            sent: Mutex::new(Vec::new()),
            started_tx,
        };
        (Arc::new(mock), started_rx)
    }

    pub fn push(&self, step: Step) {
        self.script.lock().unwrap().push_back(step);
    }

    /// The `component` of every report `send` was called with, in order.
    pub fn sent(&self) -> Vec<String> {
        let sent = self.sent.lock().unwrap();
        sent.iter().map(|r| r.metadata.component.clone()).collect()
    }

    /// Every report `send` was called with, in order.
    pub fn reports(&self) -> Vec<Report> {
        self.sent.lock().unwrap().clone()
    }
}

fn request_failed(status: u16) -> CalibrationsErr {
    CalibrationsErr::HTTPErr(HTTPErr::RequestFailed(RequestFailed {
        request: Params::get("http://mock/calibrations").meta().unwrap(),
        status: reqwest::StatusCode::from_u16(status).unwrap(),
        error: None,
        trace: miru_agent::trace!(),
    }))
}

impl Sender for MockCalibrationSender {
    async fn send(&self, report: &Report) -> Result<(), CalibrationsErr> {
        self.sent.lock().unwrap().push(report.clone());
        let step = self.script.lock().unwrap().pop_front();
        // both guards are dropped: never hold a std Mutex across an await
        let _ = self.started_tx.send(());
        match step {
            None | Some(Step::Ok) => Ok(()),
            Some(Step::Terminal) => Err(request_failed(422)),
            Some(Step::Server) => Err(request_failed(500)),
            Some(Step::Network) => Err(CalibrationsErr::HTTPErr(HTTPErr::MockErr(MockErr {
                is_network_conn_err: true,
            }))),
            Some(Step::Hang(rx)) => {
                let _ = rx.await;
                Ok(())
            }
        }
    }
}
