// standard crates
use std::sync::Arc;

// internal crates
use crate::{
    filesys::{errors::*, file::File, Atomic, Overwrite, WriteOptions},
    models::Patch,
    trace,
};

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

// ============================== SINGLE THREADED ================================== //
#[derive(Debug)]
pub struct SingleThreadCachedFile<ContentT, PatchT>
where
    ContentT: Clone + Serialize + DeserializeOwned + Patch<PatchT> + PartialEq,
{
    pub file: File,
    cache: Arc<ContentT>,
    _phantom: std::marker::PhantomData<PatchT>,
}

impl<ContentT, PatchT> SingleThreadCachedFile<ContentT, PatchT>
where
    ContentT: Clone + Serialize + DeserializeOwned + Patch<PatchT> + PartialEq,
{
    pub async fn new(file: File) -> Result<Self, FileSysErr> {
        let cache = file.read_json::<ContentT>().await?;

        // initialize the struct with the read data
        let cached_file = Self {
            file,
            cache: Arc::new(cache),
            _phantom: std::marker::PhantomData,
        };
        Ok(cached_file)
    }

    pub async fn new_with_default(file: File, default: ContentT) -> Result<Self, FileSysErr>
    where
        Self: Sized,
    {
        let result = Self::new(file.clone()).await;
        match result {
            Ok(cached_file) => Ok(cached_file),
            Err(_) => Self::create(file, &default, Overwrite::Allow).await,
        }
    }

    pub async fn create(
        file: File,
        data: &ContentT,
        overwrite: Overwrite,
    ) -> Result<Self, FileSysErr>
    where
        Self: Sized,
    {
        file.write_json(
            data,
            WriteOptions {
                overwrite,
                atomic: Atomic::Yes,
            },
        )
        .await?;
        Self::new(file).await
    }

    pub async fn read(&self) -> Arc<ContentT> {
        self.cache.clone()
    }

    pub async fn write(&mut self, data: ContentT) -> Result<(), FileSysErr> {
        self.file
            .write_json(&data, WriteOptions::OVERWRITE_ATOMIC)
            .await?;
        self.cache = Arc::new(data);
        Ok(())
    }

    pub async fn patch(&mut self, patch: PatchT) -> Result<(), FileSysErr> {
        let copy = (*self.cache).clone();
        let mut content = (*self.cache).clone();
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
    pub file: SingleThreadCachedFile<ContentT, PatchT>,
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
                    dispatch!(
                        self.file.read().await,
                        respond_to,
                        "Actor failed to read file"
                    );
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
pub struct ConcurrentCachedFile<ContentT, PatchT>
where
    PatchT: ConcurrentPatchT,
    ContentT: ConcurrentContentT<PatchT>,
{
    sender: Sender<Command<ContentT, PatchT>>,
}

impl<ContentT, PatchT> ConcurrentCachedFile<ContentT, PatchT>
where
    PatchT: ConcurrentPatchT,
    ContentT: ConcurrentContentT<PatchT>,
{
    pub async fn spawn(
        buffer_size: usize,
        file: File,
    ) -> Result<(Self, JoinHandle<()>), FileSysErr> {
        let (sender, receiver) = mpsc::channel(buffer_size);
        let worker = Worker {
            file: SingleThreadCachedFile::new(file).await?,
            receiver,
        };
        let worker_handle = tokio::spawn(worker.run());
        Ok((Self { sender }, worker_handle))
    }

    pub async fn spawn_with_default(
        buffer_size: usize,
        file: File,
        default: ContentT,
    ) -> Result<(Self, JoinHandle<()>), FileSysErr> {
        let (sender, receiver) = mpsc::channel(buffer_size);
        let worker = Worker {
            file: SingleThreadCachedFile::new_with_default(file, default).await?,
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
            "{} cached file shutdown complete",
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
