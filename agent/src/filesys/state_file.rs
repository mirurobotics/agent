// standard crates
use std::sync::Arc;

// internal crates
use crate::filesys::{errors::*, file::File, files, Atomic, Overwrite, WriteOptions};
use crate::models::Patch;
use crate::trace;

// external crates
use serde::{de::DeserializeOwned, Serialize};
use tokio::sync::{
    mpsc::{self, Receiver, Sender},
    oneshot,
};
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

/// Options for opening a state file.
pub struct Options<ContentT> {
    /// If the file is absent/unreadable, create it with this value. `None` means
    /// the file must already exist (absent is an error).
    pub default: Option<ContentT>,
    /// Permission bits applied on every write. `None` leaves the umask/atomicwrites
    /// default; `Some(0o600)` restricts secrets like the auth token.
    pub mode: Option<u32>,
}

impl<ContentT> Default for Options<ContentT> {
    fn default() -> Self {
        Self {
            default: None,
            mode: None,
        }
    }
}

// ============================== SINGLE THREADED ================================== //
#[derive(Debug)]
pub struct SingleThreadStateFile<ContentT, PatchT>
where
    ContentT: Clone + Serialize + DeserializeOwned + Patch<PatchT> + PartialEq,
{
    pub file: File,
    state: Arc<ContentT>,
    /// Permission bits applied on every write. `None` leaves the file at the
    /// umask/atomicwrites default; `Some(m)` (e.g. `0o600` for secrets like the
    /// auth token) restricts the file on both create and update.
    mode: Option<u32>,
    _phantom: std::marker::PhantomData<PatchT>,
}

impl<ContentT, PatchT> SingleThreadStateFile<ContentT, PatchT>
where
    ContentT: Clone + Serialize + DeserializeOwned + Patch<PatchT> + PartialEq,
{
    /// Open the state file described by `opts`. On a successful read the file is
    /// loaded as-is. If the read fails and `opts.default` is set, the file is
    /// created with that value (atomic, `opts.mode`) and reloaded; otherwise the
    /// read error propagates. `opts.mode` is applied on every subsequent write.
    pub async fn open(file: File, opts: Options<ContentT>) -> Result<Self, FileSysErr> {
        match Self::load(file.clone(), opts.mode).await {
            Ok(state_file) => Ok(state_file),
            Err(read_err) => {
                let Some(default) = opts.default else {
                    return Err(read_err);
                };
                files::write_json(
                    &file,
                    &default,
                    WriteOptions {
                        overwrite: Overwrite::Allow,
                        atomic: Atomic::Yes,
                        mode: opts.mode,
                    },
                )
                .await?;
                Self::load(file, opts.mode).await
            }
        }
    }

    async fn load(file: File, mode: Option<u32>) -> Result<Self, FileSysErr> {
        let state = files::read_json::<ContentT>(&file).await?;
        Ok(Self {
            file,
            state: Arc::new(state),
            mode,
            _phantom: std::marker::PhantomData,
        })
    }

    pub fn read(&self) -> Arc<ContentT> {
        self.state.clone()
    }

    pub async fn write(&mut self, data: ContentT) -> Result<(), FileSysErr> {
        files::write_json(
            &self.file,
            &data,
            WriteOptions {
                overwrite: Overwrite::Allow,
                atomic: Atomic::Yes,
                mode: self.mode,
            },
        )
        .await?;
        self.state = Arc::new(data);
        Ok(())
    }

    pub async fn patch(&mut self, patch: PatchT) -> Result<(), FileSysErr> {
        let copy = (*self.state).clone();
        let mut content = (*self.state).clone();
        content.patch(patch);
        // only write the content if it has changed
        if content == copy {
            return Ok(());
        }
        self.write(content).await
    }
}

// ================================ CONCURRENT ===================================== //

pub trait ConcurrentPatchT: Send + Sync + 'static {}
impl<T> ConcurrentPatchT for T where T: Send + Sync + 'static {}

pub trait ConcurrentContentT<PatchT>:
    Clone + Serialize + DeserializeOwned + Patch<PatchT> + Send + Sync + 'static + PartialEq
{
}
impl<T, U> ConcurrentContentT<U> for T where
    T: Clone + Serialize + DeserializeOwned + Patch<U> + Send + Sync + 'static + PartialEq
{
}

pub enum Command<ContentT, PatchT>
where
    ContentT: Clone + Serialize + DeserializeOwned + Patch<PatchT>,
{
    Shutdown {
        respond_to: oneshot::Sender<Result<(), FileSysErr>>,
    },
    Read {
        respond_to: oneshot::Sender<Arc<ContentT>>,
    },
    Write {
        data: ContentT,
        respond_to: oneshot::Sender<Result<(), FileSysErr>>,
    },
    Patch {
        patch: PatchT,
        respond_to: oneshot::Sender<Result<(), FileSysErr>>,
    },
}

pub struct Worker<ContentT, PatchT>
where
    ContentT: Clone + Serialize + DeserializeOwned + Patch<PatchT> + PartialEq,
{
    pub file: SingleThreadStateFile<ContentT, PatchT>,
    pub receiver: Receiver<Command<ContentT, PatchT>>,
}

impl<ContentT, PatchT> Worker<ContentT, PatchT>
where
    ContentT: Clone + Serialize + DeserializeOwned + Patch<PatchT> + PartialEq,
{
    pub async fn run(mut self) {
        while let Some(cmd) = self.receiver.recv().await {
            match cmd {
                Command::Shutdown { respond_to } => {
                    if let Err(e) = respond_to.send(Ok(())) {
                        error!("Actor failed to send shutdown response: {:?}", e);
                    }
                    break;
                }
                Command::Read { respond_to } => {
                    dispatch!(self.file.read(), respond_to, "Actor failed to read file");
                }
                Command::Write { data, respond_to } => {
                    dispatch!(
                        self.file.write(data).await,
                        respond_to,
                        "Actor failed to write file"
                    );
                }
                Command::Patch { patch, respond_to } => {
                    dispatch!(
                        self.file.patch(patch).await,
                        respond_to,
                        "Actor failed to patch file"
                    );
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct ConcurrentStateFile<ContentT, PatchT>
where
    PatchT: ConcurrentPatchT,
    ContentT: ConcurrentContentT<PatchT>,
{
    sender: Sender<Command<ContentT, PatchT>>,
}

impl<ContentT, PatchT> ConcurrentStateFile<ContentT, PatchT>
where
    PatchT: ConcurrentPatchT,
    ContentT: ConcurrentContentT<PatchT>,
{
    pub async fn spawn(
        buffer_size: usize,
        file: File,
        opts: Options<ContentT>,
    ) -> Result<(Self, JoinHandle<()>), FileSysErr> {
        let (sender, receiver) = mpsc::channel(buffer_size);
        let worker = Worker {
            file: SingleThreadStateFile::open(file, opts).await?,
            receiver,
        };
        let worker_handle = tokio::spawn(worker.run());
        Ok((Self { sender }, worker_handle))
    }

    async fn send_command<R>(
        &self,
        op: &str,
        make_cmd: impl FnOnce(oneshot::Sender<R>) -> Command<ContentT, PatchT>,
    ) -> Result<R, FileSysErr> {
        let (send, recv) = oneshot::channel();
        self.sender.send(make_cmd(send)).await.map_err(|e| {
            error!("Failed to send {op} command to actor: {e:?}");
            FileSysErr::SendActorMessageErr(SendActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })?;
        recv.await.map_err(|e| {
            error!("Failed to receive {op} response from actor: {e:?}");
            FileSysErr::ReceiveActorMessageErr(ReceiveActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })
    }

    pub async fn shutdown(&self) -> Result<(), FileSysErr> {
        self.send_command("shutdown", |tx| Command::Shutdown { respond_to: tx })
            .await??;
        info!(
            "{} state file shutdown complete",
            std::any::type_name::<ContentT>()
        );
        Ok(())
    }

    pub async fn read(&self) -> Result<Arc<ContentT>, FileSysErr> {
        self.send_command("read", |tx| Command::Read { respond_to: tx })
            .await
    }

    pub async fn write(&self, data: ContentT) -> Result<(), FileSysErr> {
        self.send_command("write", |tx| Command::Write {
            data,
            respond_to: tx,
        })
        .await?
    }

    pub async fn patch(&self, patch: PatchT) -> Result<(), FileSysErr> {
        self.send_command("patch", |tx| Command::Patch {
            patch,
            respond_to: tx,
        })
        .await?
    }
}
