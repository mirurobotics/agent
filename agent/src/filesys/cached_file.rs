// standard library
use std::sync::Arc;

// internal crates
use crate::{
    filesys::{errors::*, file::File},
    trace,
    utils::Mergeable,
};

// external crates
use serde::{de::DeserializeOwned, Serialize};
use tokio::sync::{
    mpsc::{self, Receiver, Sender},
    oneshot,
};
use tokio::task::JoinHandle;
use tracing::{error, info};

// ============================== SINGLE THREADED ================================== //
#[derive(Debug)]
pub struct SingleThreadCachedFile<ContentT, UpdatesT>
where
    ContentT: Clone + Serialize + DeserializeOwned + Mergeable<UpdatesT> + PartialEq,
{
    pub file: File,
    cache: Arc<ContentT>,
    _phantom: std::marker::PhantomData<UpdatesT>,
}

impl<ContentT, UpdatesT> SingleThreadCachedFile<ContentT, UpdatesT>
where
    ContentT: Clone + Serialize + DeserializeOwned + Mergeable<UpdatesT> + PartialEq,
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
            Err(_) => Self::create(file, &default, true).await,
        }
    }

    pub async fn create(file: File, data: &ContentT, overwrite: bool) -> Result<Self, FileSysErr>
    where
        Self: Sized,
    {
        file.write_json(data, overwrite, true).await?;
        Self::new(file).await
    }

    pub async fn read(&self) -> Arc<ContentT> {
        self.cache.clone()
    }

    pub async fn write(&mut self, data: ContentT) -> Result<(), FileSysErr> {
        self.file.write_json(&data, true, true).await?;
        self.cache = Arc::new(data);
        Ok(())
    }

    pub async fn patch(&mut self, updates: UpdatesT) -> Result<(), FileSysErr> {
        let copy = (*self.cache).clone();
        let mut content = (*self.cache).clone();
        content.merge(updates);
        // only write the content if it has changed
        if content == copy {
            return Ok(());
        }
        self.write(content).await
    }
}

// ================================ CONCURRENT ===================================== //

pub trait ConcurrentUpdatesT: Send + Sync + 'static {}
impl<T> ConcurrentUpdatesT for T where T: Send + Sync + 'static {}

pub trait ConcurrentContentT<UpdatesT>:
    Clone + Serialize + DeserializeOwned + Mergeable<UpdatesT> + Send + Sync + 'static + PartialEq
{
}
impl<T, U> ConcurrentContentT<U> for T where
    T: Clone + Serialize + DeserializeOwned + Mergeable<U> + Send + Sync + 'static + PartialEq
{
}

pub enum WorkerCommand<ContentT, UpdatesT>
where
    ContentT: Clone + Serialize + DeserializeOwned + Mergeable<UpdatesT>,
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
        updates: UpdatesT,
        respond_to: oneshot::Sender<Result<(), FileSysErr>>,
    },
}

pub struct Worker<ContentT, UpdatesT>
where
    ContentT: Clone + Serialize + DeserializeOwned + Mergeable<UpdatesT> + PartialEq,
{
    pub file: SingleThreadCachedFile<ContentT, UpdatesT>,
    pub receiver: Receiver<WorkerCommand<ContentT, UpdatesT>>,
}

impl<ContentT, UpdatesT> Worker<ContentT, UpdatesT>
where
    ContentT: Clone + Serialize + DeserializeOwned + Mergeable<UpdatesT> + PartialEq,
{
    pub async fn run(mut self) {
        while let Some(cmd) = self.receiver.recv().await {
            match cmd {
                WorkerCommand::Shutdown { respond_to } => {
                    if let Err(e) = respond_to.send(Ok(())) {
                        error!("Actor failed to send shutdown response: {:?}", e);
                    }
                    break;
                }
                WorkerCommand::Read { respond_to } => {
                    let result = self.file.read().await;
                    if respond_to.send(result).is_err() {
                        error!("Actor failed to read file");
                    }
                }
                WorkerCommand::Write { data, respond_to } => {
                    let result = self.file.write(data).await;
                    if respond_to.send(result).is_err() {
                        error!("Actor failed to write file");
                    }
                }
                WorkerCommand::Patch {
                    updates,
                    respond_to,
                } => {
                    let result = self.file.patch(updates).await;
                    if respond_to.send(result).is_err() {
                        error!("Actor failed to patch file");
                    }
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct ConcurrentCachedFile<ContentT, UpdatesT>
where
    UpdatesT: ConcurrentUpdatesT,
    ContentT: ConcurrentContentT<UpdatesT>,
{
    sender: Sender<WorkerCommand<ContentT, UpdatesT>>,
}

impl<ContentT, UpdatesT> ConcurrentCachedFile<ContentT, UpdatesT>
where
    UpdatesT: ConcurrentUpdatesT,
    ContentT: ConcurrentContentT<UpdatesT>,
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

    pub async fn shutdown(&self) -> Result<(), FileSysErr> {
        let (send, recv) = oneshot::channel();
        self.sender
            .send(WorkerCommand::Shutdown { respond_to: send })
            .await
            .map_err(|e| {
                error!("Failed to send shutdown command to actor: {:?}", e);
                FileSysErr::SendActorMessageErr(SendActorMessageErr {
                    source: Box::new(e),
                    trace: trace!(),
                })
            })?;
        recv.await.map_err(|e| {
            error!("Failed to receive shutdown response from actor: {:?}", e);
            FileSysErr::ReceiveActorMessageErr(ReceiveActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })??;
        info!(
            "{} cached file shutdown complete",
            std::any::type_name::<ContentT>()
        );
        Ok(())
    }

    pub async fn read(&self) -> Result<Arc<ContentT>, FileSysErr> {
        let (send, recv) = oneshot::channel();
        self.sender
            .send(WorkerCommand::Read { respond_to: send })
            .await
            .map_err(|e| {
                error!("Failed to send read command to actor: {:?}", e);
                FileSysErr::SendActorMessageErr(SendActorMessageErr {
                    source: Box::new(e),
                    trace: trace!(),
                })
            })?;
        recv.await.map_err(|e| {
            error!("Failed to receive read response from actor: {:?}", e);
            FileSysErr::ReceiveActorMessageErr(ReceiveActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })
    }

    pub async fn write(&self, data: ContentT) -> Result<(), FileSysErr> {
        let (send, recv) = oneshot::channel();
        self.sender
            .send(WorkerCommand::Write {
                data,
                respond_to: send,
            })
            .await
            .map_err(|e| {
                error!("Failed to send write command to actor: {:?}", e);
                FileSysErr::SendActorMessageErr(SendActorMessageErr {
                    source: Box::new(e),
                    trace: trace!(),
                })
            })?;
        recv.await.map_err(|e| {
            error!("Failed to receive write response from actor: {:?}", e);
            FileSysErr::ReceiveActorMessageErr(ReceiveActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })?
    }

    pub async fn patch(&self, updates: UpdatesT) -> Result<(), FileSysErr> {
        let (send, recv) = oneshot::channel();
        self.sender
            .send(WorkerCommand::Patch {
                updates,
                respond_to: send,
            })
            .await
            .map_err(|e| {
                error!("Failed to send patch command to actor: {:?}", e);
                FileSysErr::SendActorMessageErr(SendActorMessageErr {
                    source: Box::new(e),
                    trace: trace!(),
                })
            })?;
        recv.await.map_err(|e| {
            error!("Failed to receive patch response from actor: {:?}", e);
            FileSysErr::ReceiveActorMessageErr(ReceiveActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })?
    }
}
