// standard library
use std::sync::Arc;

// internal crates
use crate::authn::{errors::*, token, token::Token};
use crate::crypt::{base64, rsa};
use crate::filesys::{cached_file::SingleThreadCachedFile, file::File, path::PathExt};
use crate::http;
use crate::http::devices;
use crate::trace;
use openapi_client::models::{IssueDeviceClaims, IssueDeviceTokenRequest};

// external crates
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tracing::{error, info};
use uuid::Uuid;

pub type TokenFile = SingleThreadCachedFile<Token, token::Updates>;

#[derive(Serialize)]
struct IssueTokenClaim {
    pub device_id: String,
    pub nonce: String,
    pub expiration: i64,
}

// =================================== TRAIT ======================================= //
#[allow(async_fn_in_trait)]
pub trait TokenManagerExt {
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
        // prepare the token request
        let payload = self.prepare_issue_token_request().await?;

        // send the token request
        let resp = devices::issue_token(
            self.http_client.as_ref(),
            devices::IssueTokenParams {
                id: &self.device_id,
                payload: &payload,
            },
        )
        .await?;

        // format the response
        let expires_at = resp.expires_at.parse::<DateTime<Utc>>().map_err(|e| {
            AuthnErr::TimestampConversionErr(TimestampConversionErr {
                msg: format!(
                    "failed to parse date time '{}' from string: {}",
                    resp.expires_at, e
                ),
                trace: trace!(),
            })
        })?;
        Ok(Token {
            token: resp.token,
            expires_at,
        })
    }

    async fn prepare_issue_token_request(&self) -> Result<IssueDeviceTokenRequest, AuthnErr> {
        // prepare the claims
        let nonce = Uuid::new_v4().to_string();
        let expiration = Utc::now() + Duration::minutes(2);
        let claims = IssueTokenClaim {
            device_id: self.device_id.to_string(),
            nonce: nonce.clone(),
            expiration: expiration.timestamp(),
        };

        // serialize the claims into a JSON byte vector
        let claims_bytes = serde_json::to_vec(&claims).map_err(|e| {
            AuthnErr::SerdeErr(SerdeErr {
                source: e,
                trace: trace!(),
            })
        })?;

        // sign the claims
        let signature_bytes = rsa::sign(&self.private_key_file, &claims_bytes).await?;
        let signature = base64::encode_bytes_standard(&signature_bytes);

        // convert it to the http client format
        let claims = IssueDeviceClaims {
            device_id: self.device_id.to_string(),
            nonce,
            expiration: expiration.to_rfc3339(),
        };

        Ok(IssueDeviceTokenRequest {
            claims: Box::new(claims),
            signature,
        })
    }
}

// ========================= MULTI-THREADED IMPLEMENTATION ========================= //
pub(crate) enum WorkerCommand {
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
    receiver: Receiver<WorkerCommand>,
}

impl<HTTPClientT: http::ClientI> Worker<HTTPClientT> {
    pub(crate) async fn run(mut self) {
        while let Some(cmd) = self.receiver.recv().await {
            match cmd {
                WorkerCommand::Shutdown { respond_to } => {
                    if respond_to.send(Ok(())).is_err() {
                        error!("Actor failed to send shutdown response");
                    }
                    break;
                }
                WorkerCommand::GetToken { respond_to } => {
                    let token = self.token_mngr.get_token().await;
                    if respond_to.send(Ok(token)).is_err() {
                        error!("Actor failed to send token");
                    }
                }
                WorkerCommand::RefreshToken { respond_to } => {
                    let result = self.token_mngr.refresh_token().await;
                    if respond_to.send(result).is_err() {
                        error!("Actor failed to refresh token");
                    }
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct TokenManager {
    sender: Sender<WorkerCommand>,
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
        make_cmd: impl FnOnce(oneshot::Sender<R>) -> WorkerCommand,
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
        self.send_command("shutdown", |tx| WorkerCommand::Shutdown { respond_to: tx })
            .await??;
        info!("Token manager shutdown complete");
        Ok(())
    }

    async fn get_token(&self) -> Result<Arc<Token>, AuthnErr> {
        self.send_command("get_token", |tx| WorkerCommand::GetToken { respond_to: tx })
            .await?
    }

    async fn refresh_token(&self) -> Result<(), AuthnErr> {
        self.send_command("refresh_token", |tx| WorkerCommand::RefreshToken {
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
