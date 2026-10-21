// standard crates
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::cooldown;
use crate::filesys::{errors::*, file::File, files, Atomic, Overwrite, WriteOptions};
use crate::models::Patch;

// external crates
use serde::{de::DeserializeOwned, Serialize};
use tracing::error;

/// Waits between retries of a state file read that failed for a reason other
/// than "missing" or "unparseable" (e.g. a transient EIO).
const READ_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_millis(50),
    Duration::from_millis(250),
    Duration::from_millis(1000),
];

/// Single-threaded state file: a JSON file plus its cached contents.
/// [`crate::filesys::state_file::StateFile`] wraps it in an actor for shared use.
#[derive(Debug)]
pub struct StateFileImpl<ContentT, PatchT>
where
    ContentT: Clone + Serialize + DeserializeOwned + Patch<PatchT> + PartialEq,
{
    pub file: File,
    state: Arc<ContentT>,
    _phantom: std::marker::PhantomData<PatchT>,
}

impl<ContentT, PatchT> StateFileImpl<ContentT, PatchT>
where
    ContentT: Clone + Serialize + DeserializeOwned + Patch<PatchT> + PartialEq,
{
    pub async fn new(file: File) -> Result<Self, FileSysErr> {
        let state = files::read_json::<ContentT>(&file).await?;

        // initialize the struct with the read data
        let state_file = Self {
            file,
            state: Arc::new(state),
            _phantom: std::marker::PhantomData,
        };
        Ok(state_file)
    }

    /// Opens `file`:
    ///
    /// - **Missing:** created with `default`.
    /// - **Unparseable:** copied aside to `<name>.corrupt-<uuid>` (best
    ///   effort), then atomically replaced with `default`.
    /// - **Other read error:** retried per [`READ_RETRY_DELAYS`], then returned
    ///   with the file left untouched.
    pub async fn new_with_default(file: File, default: ContentT) -> Result<Self, FileSysErr>
    where
        Self: Sized,
    {
        match Self::new_with_retry(&file).await {
            Ok(state_file) => Ok(state_file),
            Err(FileSysErr::PathDoesNotExistErr(_)) => {
                Self::create(file, &default, Overwrite::Allow).await
            }
            Err(FileSysErr::ParseJSONErr(err)) => {
                match Self::copy_aside(&file).await {
                    Ok(aside) => error!(
                        "state file {file} could not be parsed ({err}); copied it to {aside} \
                         and started from the default"
                    ),
                    Err(copy_err) => error!(
                        "state file {file} could not be parsed ({err}) or copied aside \
                         ({copy_err}); started from the default"
                    ),
                }
                Self::create(file, &default, Overwrite::Allow).await
            }
            Err(err) => Err(err),
        }
    }

    /// [`Self::new`], retrying errors other than "missing" and "unparseable"
    /// after each of [`READ_RETRY_DELAYS`].
    async fn new_with_retry(file: &File) -> Result<Self, FileSysErr> {
        let retryable = |err: &FileSysErr| {
            !matches!(
                err,
                FileSysErr::PathDoesNotExistErr(_) | FileSysErr::ParseJSONErr(_)
            )
        };
        cooldown::retry(
            |n| READ_RETRY_DELAYS.get(n as usize).copied(),
            retryable,
            tokio::time::sleep,
            || Self::new(file.clone()),
        )
        .await
    }

    /// Copies `file` to a uniquely named `.corrupt-<uuid>` sibling, keeping
    /// its permissions.
    async fn copy_aside(file: &File) -> Result<File, FileSysErr> {
        let aside = file.parent()?.file(&format!(
            "{}.corrupt-{}",
            file.name()?,
            uuid::Uuid::new_v4()
        ));
        files::copy_to(file, &aside, Overwrite::Deny).await?;
        Ok(aside)
    }

    pub async fn create(
        file: File,
        data: &ContentT,
        overwrite: Overwrite,
    ) -> Result<Self, FileSysErr>
    where
        Self: Sized,
    {
        files::write_json(
            &file,
            data,
            WriteOptions {
                overwrite,
                atomic: Atomic::Yes,
                mode: None,
            },
        )
        .await?;
        Self::new(file).await
    }

    pub fn read(&self) -> Arc<ContentT> {
        self.state.clone()
    }

    pub async fn write(&mut self, data: ContentT) -> Result<(), FileSysErr> {
        files::write_json(&self.file, &data, WriteOptions::OVERWRITE_ATOMIC).await?;
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
