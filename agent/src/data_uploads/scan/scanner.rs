// internal crates
pub use crate::data_uploads::scan::state::{Config, StableFile};
use crate::data_uploads::scan::{
    errors::*,
    scanner_impl::{ScannerArgs, ScannerImpl},
};
use crate::models::{Deployment, FileRule};
use crate::trace;

// external crates
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{error, info};

macro_rules! dispatch {
    ($op:expr, $respond_to:expr, $msg:expr) => {{
        let result = $op;
        if $respond_to.send(result).is_err() {
            error!($msg);
        }
    }};
}

#[allow(async_fn_in_trait)]
pub trait ScannerExt {
    async fn clear_rules(&self) -> Result<(), ScanErr>;
    async fn update_rules(
        &self,
        deployment: Deployment,
        rules: Vec<FileRule>,
    ) -> Result<(), ScanErr>;
    async fn scan(&self) -> Result<(), ScanErr>;
    async fn shutdown(&self) -> Result<(), ScanErr>;
}

pub enum Command {
    ClearRules {
        respond_to: oneshot::Sender<Result<(), ScanErr>>,
    },
    UpdateRules {
        deployment: Box<Deployment>,
        rules: Vec<FileRule>,
        respond_to: oneshot::Sender<Result<(), ScanErr>>,
    },
    Scan {
        respond_to: oneshot::Sender<Result<(), ScanErr>>,
    },
    Shutdown {
        respond_to: oneshot::Sender<Result<(), ScanErr>>,
    },
    #[cfg(test)]
    GetRules {
        respond_to: oneshot::Sender<Result<Vec<FileRule>, ScanErr>>,
    },
    #[cfg(test)]
    GetLedgerCount {
        respond_to: oneshot::Sender<Result<usize, ScanErr>>,
    },
}

pub struct Worker {
    scanner: ScannerImpl,
    receiver: mpsc::Receiver<Command>,
}

impl Worker {
    pub fn new(scanner: ScannerImpl, receiver: mpsc::Receiver<Command>) -> Self {
        Self { scanner, receiver }
    }

    // lint:allow(funclen) — actor dispatch table; one arm per Command variant
    pub async fn run(mut self) {
        while let Some(cmd) = self.receiver.recv().await {
            match cmd {
                Command::Shutdown { respond_to } => {
                    if let Err(e) = respond_to.send(Ok(())) {
                        error!("Actor failed to send shutdown response: {:?}", e);
                    }
                    break;
                }
                Command::ClearRules { respond_to } => {
                    dispatch!(
                        self.scanner.clear_rules().await,
                        respond_to,
                        "Actor failed to send clear rules response"
                    );
                }
                Command::UpdateRules {
                    deployment,
                    rules,
                    respond_to,
                } => {
                    dispatch!(
                        self.scanner.update_rules(*deployment, rules).await,
                        respond_to,
                        "Actor failed to send update configs response"
                    );
                }
                Command::Scan { respond_to } => {
                    dispatch!(
                        self.scanner.scan().await,
                        respond_to,
                        "Actor failed to send scan response"
                    );
                }
                #[cfg(test)]
                Command::GetRules { respond_to } => {
                    dispatch!(
                        self.scanner.get_rules().await,
                        respond_to,
                        "Actor failed to send get rules response"
                    );
                }
                #[cfg(test)]
                Command::GetLedgerCount { respond_to } => {
                    dispatch!(
                        self.scanner.get_ledger_count().await,
                        respond_to,
                        "Actor failed to send get ledger count response"
                    );
                }
            }
        }
    }
}

/// Command handle to the [`ScannerImpl`] actor. Reactive, not
/// self-scheduling: each [`scan`](ScannerExt::scan) call performs exactly one
/// discover/evaluate pass and reports newly-stable files exactly once. The
/// cadence that drives repeated passes is imposed by an external driver (future
/// PR), not by this type.
#[derive(Debug)]
pub struct Scanner {
    sender: mpsc::Sender<Command>,
}

impl Scanner {
    pub fn spawn(buffer_size: usize, args: ScannerArgs) -> Result<(Self, JoinHandle<()>), ScanErr> {
        let (sender, receiver) = mpsc::channel(buffer_size);
        let worker = Worker {
            scanner: ScannerImpl::new(args)?,
            receiver,
        };
        let worker_handle = tokio::spawn(worker.run());
        Ok((Self { sender }, worker_handle))
    }

    pub fn new(sender: mpsc::Sender<Command>) -> Self {
        Self { sender }
    }

    async fn send_command<R>(
        &self,
        cmd: impl FnOnce(oneshot::Sender<R>) -> Command,
    ) -> Result<R, ScanErr> {
        let (send, recv) = oneshot::channel();
        self.sender.send(cmd(send)).await.map_err(|e| {
            ScanErr::SendActorMessageErr(SendActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })?;
        recv.await.map_err(|e| {
            ScanErr::ReceiveActorMessageErr(ReceiveActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })
    }

    #[cfg(test)]
    pub(crate) async fn get_rules(&self) -> Result<Vec<FileRule>, ScanErr> {
        self.send_command(|tx| Command::GetRules { respond_to: tx })
            .await?
    }

    #[cfg(test)]
    pub(crate) async fn get_ledger_count(&self) -> Result<usize, ScanErr> {
        self.send_command(|tx| Command::GetLedgerCount { respond_to: tx })
            .await?
    }
}

impl ScannerExt for Scanner {
    async fn clear_rules(&self) -> Result<(), ScanErr> {
        self.send_command(|tx| Command::ClearRules { respond_to: tx })
            .await?
    }

    async fn update_rules(
        &self,
        deployment: Deployment,
        rules: Vec<FileRule>,
    ) -> Result<(), ScanErr> {
        self.send_command(|tx| Command::UpdateRules {
            deployment: Box::new(deployment),
            rules,
            respond_to: tx,
        })
        .await?
    }

    async fn scan(&self) -> Result<(), ScanErr> {
        self.send_command(|tx| Command::Scan { respond_to: tx })
            .await?
    }

    async fn shutdown(&self) -> Result<(), ScanErr> {
        self.send_command(|tx| Command::Shutdown { respond_to: tx })
            .await??;
        info!("Scanner shutdown complete");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    // internal crates
    use super::{Scanner, ScannerArgs, ScannerExt, ScannerImpl, Worker};
    use crate::data_uploads::scan::ScanErr;
    use crate::models::{Deployment, DplActivity, FileRule, FileRuleSource};

    // external crates
    use tokio::sync::mpsc;

    // Scanner logic (rules, scanning, snapshots, sinks) is tested against
    // ScannerImpl directly in scanner_impl.rs. These tests cover the actor:
    // the handle, the worker loop, and shutdown.

    fn deployment(id: &str) -> Deployment {
        Deployment {
            id: id.to_string(),
            activity_status: DplActivity::Deployed,
            release_id: id.to_string(),
            ..Default::default()
        }
    }

    fn rule(rule_id: &str) -> FileRule {
        FileRule {
            id: rule_id.to_string(),
            source: FileRuleSource {
                glob: "/none/*.mcap".to_string(),
                stability_window_secs: 0,
            },
            ..Default::default()
        }
    }

    fn rule_ids(rules: &[FileRule]) -> Vec<String> {
        let mut ids: Vec<String> = rules.iter().map(|r| r.id.clone()).collect();
        ids.sort();
        ids
    }

    mod construction {
        use super::*;

        #[tokio::test]
        async fn worker_new_and_scanner_new_round_trip() {
            let (tx, rx) = mpsc::channel(64);
            let scanner_impl = ScannerImpl::new(ScannerArgs::default()).unwrap();
            let worker = Worker::new(scanner_impl, rx);
            let handle = tokio::spawn(worker.run());

            let scanner = Scanner::new(tx);

            // the hand-built actor answers commands: empty ledger.
            assert_eq!(scanner.get_ledger_count().await.unwrap(), 0);

            // and shuts down cleanly; later commands then error on a closed channel.
            scanner.shutdown().await.unwrap();
            handle.await.unwrap();
            let err = scanner.get_ledger_count().await.unwrap_err();
            assert!(matches!(err, ScanErr::SendActorMessageErr(_)));
        }
    }

    mod commands {
        use super::*;

        // Each command reaches the ScannerImpl behind the actor and its result
        // comes back through the handle.
        #[tokio::test]
        async fn commands_reach_scanner_impl() {
            let (scanner, _handle) = Scanner::spawn(64, ScannerArgs::default()).unwrap();

            scanner
                .update_rules(deployment("d"), vec![rule("a"), rule("b")])
                .await
                .unwrap();
            assert_eq!(
                rule_ids(&scanner.get_rules().await.unwrap()),
                vec!["a".to_string(), "b".to_string()]
            );

            scanner.scan().await.unwrap();
            assert_eq!(scanner.get_ledger_count().await.unwrap(), 0);

            // cleared rules have no candidates, so the next scan prunes them.
            scanner.clear_rules().await.unwrap();
            scanner.scan().await.unwrap();
            assert!(scanner.get_rules().await.unwrap().is_empty());
        }

        // An error from ScannerImpl comes back through the handle unchanged.
        #[tokio::test]
        async fn command_error_is_returned_to_caller() {
            let (scanner, _handle) = Scanner::spawn(64, ScannerArgs::default()).unwrap();

            let err = scanner
                .update_rules(deployment("d"), vec![rule("dup"), rule("dup")])
                .await
                .unwrap_err();
            assert!(matches!(err, ScanErr::DuplicateFileRuleID(ref e) if e.file_rule_id == "dup"));
        }
    }

    mod shutdown {
        use super::*;

        // shutdown stops the actor loop; the task completes and later commands error.
        #[tokio::test]
        async fn shutdown_stops_actor_and_later_commands_error() {
            let (scanner, handle) = Scanner::spawn(64, ScannerArgs::default()).unwrap();

            scanner.shutdown().await.unwrap();
            handle.await.unwrap();

            let err = scanner.scan().await.unwrap_err();
            assert!(matches!(err, ScanErr::SendActorMessageErr(_)));
        }
    }
}
