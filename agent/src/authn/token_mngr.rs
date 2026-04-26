// standard crates
use std::sync::Arc;

// internal crates
use crate::authn::{errors::*, issue, token, token::Token};
use crate::filesys::{cached_file::SingleThreadCachedFile, file::File, path::PathExt};
use crate::http;
use crate::trace;

// external crates
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::oneshot;
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

pub type TokenFile = SingleThreadCachedFile<Token, token::Updates>;

// =================================== TRAIT ======================================= //
#[allow(async_fn_in_trait)]
pub trait TokenManagerExt: Send + Sync {
    async fn shutdown(&self) -> Result<(), AuthnErr>;
    async fn get_token(&self) -> Result<Arc<Token>, AuthnErr>;
    async fn refresh_token(&self) -> Result<(), AuthnErr>;
}

// ======================== SINGLE THREADED IMPLEMENTATION ========================= //
pub(crate) struct SingleThreadTokenManager<HTTPClientT: http::ClientI> {
    device_id: String,
    http_client: Arc<HTTPClientT>,
    token_file: TokenFile,
    private_key_file: File,
}

impl<HTTPClientT: http::ClientI> SingleThreadTokenManager<HTTPClientT> {
    pub(crate) fn new(
        device_id: String,
        http_client: Arc<HTTPClientT>,
        token_file: TokenFile,
        private_key_file: File,
    ) -> Result<Self, AuthnErr> {
        token_file.file.assert_exists()?;
        private_key_file.assert_exists()?;
        Ok(Self {
            device_id,
            http_client,
            token_file,
            private_key_file,
        })
    }

    async fn get_token(&self) -> Arc<Token> {
        // get the token
        self.token_file.read().await
    }

    async fn refresh_token(&mut self) -> Result<(), AuthnErr> {
        // attempt to issue a new token
        let token = self.issue_token().await?;

        // update the token file
        self.token_file.write(token).await?;

        Ok(())
    }

    async fn issue_token(&self) -> Result<Token, AuthnErr> {
        issue::issue_token(
            self.http_client.as_ref(),
            &self.private_key_file,
            &self.device_id,
        )
        .await
    }
}

// ========================= MULTI-THREADED IMPLEMENTATION ========================= //
pub(crate) enum Command {
    GetToken {
        respond_to: oneshot::Sender<Result<Arc<Token>, AuthnErr>>,
    },
    RefreshToken {
        respond_to: oneshot::Sender<Result<(), AuthnErr>>,
    },
    Shutdown {
        respond_to: oneshot::Sender<Result<(), AuthnErr>>,
    },
}

pub(crate) struct Worker<HTTPClientT: http::ClientI> {
    token_mngr: SingleThreadTokenManager<HTTPClientT>,
    receiver: Receiver<Command>,
}

impl<HTTPClientT: http::ClientI> Worker<HTTPClientT> {
    pub(crate) async fn run(mut self) {
        while let Some(cmd) = self.receiver.recv().await {
            match cmd {
                Command::Shutdown { respond_to } => {
                    if respond_to.send(Ok(())).is_err() {
                        error!("Actor failed to send shutdown response");
                    }
                    break;
                }
                Command::GetToken { respond_to } => {
                    dispatch!(
                        Ok(self.token_mngr.get_token().await),
                        respond_to,
                        "Actor failed to send token"
                    );
                }
                Command::RefreshToken { respond_to } => {
                    dispatch!(
                        self.token_mngr.refresh_token().await,
                        respond_to,
                        "Actor failed to refresh token"
                    );
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct TokenManager {
    sender: Sender<Command>,
}

impl TokenManager {
    pub fn spawn<HTTPClientT: http::ClientI + 'static>(
        buffer_size: usize,
        device_id: String,
        http_client: Arc<HTTPClientT>,
        token_file: TokenFile,
        private_key_file: File,
    ) -> Result<(Self, JoinHandle<()>), AuthnErr> {
        let (sender, receiver) = mpsc::channel(buffer_size);
        let worker = Worker {
            token_mngr: SingleThreadTokenManager::new(
                device_id,
                http_client,
                token_file,
                private_key_file,
            )?,
            receiver,
        };
        let worker_handle = tokio::spawn(worker.run());
        Ok((Self { sender }, worker_handle))
    }

    async fn send_command<R>(
        &self,
        op: &str,
        make_cmd: impl FnOnce(oneshot::Sender<R>) -> Command,
    ) -> Result<R, AuthnErr> {
        let (send, recv) = oneshot::channel();
        self.sender.send(make_cmd(send)).await.map_err(|e| {
            error!("Failed to send {op} command to actor: {e:?}");
            AuthnErr::SendActorMessageErr(SendActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })?;
        recv.await.map_err(|e| {
            error!("Failed to receive {op} response from actor: {e:?}");
            AuthnErr::ReceiveActorMessageErr(ReceiveActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })
    }
}

impl TokenManagerExt for TokenManager {
    async fn shutdown(&self) -> Result<(), AuthnErr> {
        info!("Shutting down token manager...");
        self.send_command("shutdown", |tx| Command::Shutdown { respond_to: tx })
            .await??;
        info!("Token manager shutdown complete");
        Ok(())
    }

    async fn get_token(&self) -> Result<Arc<Token>, AuthnErr> {
        self.send_command("get_token", |tx| Command::GetToken { respond_to: tx })
            .await?
    }

    async fn refresh_token(&self) -> Result<(), AuthnErr> {
        self.send_command("refresh_token", |tx| Command::RefreshToken {
            respond_to: tx,
        })
        .await?
    }
}

impl TokenManagerExt for Arc<TokenManager> {
    async fn shutdown(&self) -> Result<(), AuthnErr> {
        self.as_ref().shutdown().await
    }

    async fn get_token(&self) -> Result<Arc<Token>, AuthnErr> {
        self.as_ref().get_token().await
    }

    async fn refresh_token(&self) -> Result<(), AuthnErr> {
        self.as_ref().refresh_token().await
    }
}
