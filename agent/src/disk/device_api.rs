//! Discovery file for TCP clients of the local device API.
//!
//! The app writes the file once the TCP listener is bound and removes it after
//! the listener stops, so the file exists only while TCP is being served. It
//! holds that listener's port and the bearer token for this agent run.
//! Clients:
//! - read the file, then close it right away;
//! - treat a missing file or a refused connection as "not serving" and retry
//!   later by re-reading the file;
//! - re-read the file on a 401, because the token changes on every start.

// standard crates
use std::future::Future;
use std::time::Duration;

// internal crates
use crate::disk::errors::DiskErr;
use crate::filesys::{self, files, Atomic, FileSysErr, Overwrite, WriteOptions};

// external crates
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use tracing::debug;

/// Windows refuses to replace or delete a file while another process has it
/// open without delete sharing, which Python's `open` does. Readers close the
/// file quickly, so a few short retries ride out the overlap. Unix has no such
/// conflict, so it tries once.
const ATTEMPTS: u32 = if cfg!(windows) { 10 } else { 1 };
const RETRY_DELAY: Duration = Duration::from_millis(50);

/// Discovery file contents. Not `Debug`, so the token cannot reach logs.
#[derive(Serialize)]
struct Discovery<'a> {
    port: u16,
    token: &'a str,
}

/// Atomically write the TCP port and bearer token for local device API
/// clients. On Unix the file mode is 0640 (owner read-write, group read); on
/// Windows the file inherits the `device-api` directory ACL.
pub async fn write(file: &filesys::File, port: u16, token: &SecretString) -> Result<(), DiskErr> {
    let discovery = Discovery {
        port,
        token: token.expose_secret(),
    };
    let opts = WriteOptions {
        overwrite: Overwrite::Allow,
        atomic: Atomic::Yes,
        mode: Some(0o640),
    };
    retry(ATTEMPTS, || files::write_json(file, &discovery, opts)).await?;
    Ok(())
}

/// Remove the discovery file; a missing file is not an error.
pub async fn remove(file: &filesys::File) -> Result<(), DiskErr> {
    retry(ATTEMPTS, || files::delete(file)).await?;
    Ok(())
}

async fn retry<F, Fut>(attempts: u32, mut op: F) -> Result<(), FileSysErr>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<(), FileSysErr>>,
{
    let mut attempt = 1;
    loop {
        match op().await {
            Err(e) if attempt < attempts => {
                debug!(
                    "Discovery file operation failed (attempt {attempt}/{attempts}), retrying: {e}"
                );
                attempt += 1;
                tokio::time::sleep(RETRY_DELAY).await;
            }
            result => return result,
        }
    }
}

#[cfg(test)]
mod tests {
    // standard crates
    use std::sync::atomic::{AtomicU32, Ordering};

    // internal crates
    use super::*;
    use crate::filesys::errors::DeleteFileErr;
    use crate::trace;

    fn transient_err() -> FileSysErr {
        FileSysErr::DeleteFileErr(DeleteFileErr {
            source: Box::new(std::io::Error::other("sharing violation")),
            file: filesys::File::new("device-api.json"),
            trace: trace!(),
        })
    }

    async fn run(attempts: u32, failures: u32) -> (Result<(), FileSysErr>, u32) {
        let calls = AtomicU32::new(0);
        let result = retry(attempts, || async {
            if calls.fetch_add(1, Ordering::SeqCst) < failures {
                Err(transient_err())
            } else {
                Ok(())
            }
        })
        .await;
        (result, calls.load(Ordering::SeqCst))
    }

    #[tokio::test]
    async fn succeeds_first_try_without_retrying() {
        let (result, calls) = run(3, 0).await;
        assert!(result.is_ok());
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn retries_until_success() {
        let (result, calls) = run(3, 2).await;
        assert!(result.is_ok());
        assert_eq!(calls, 3);
    }

    #[tokio::test]
    async fn returns_last_error_when_attempts_run_out() {
        let (result, calls) = run(3, 5).await;
        assert!(matches!(result, Err(FileSysErr::DeleteFileErr(_))));
        assert_eq!(calls, 3);
    }

    #[tokio::test]
    async fn single_attempt_does_not_retry() {
        let (result, calls) = run(1, 1).await;
        assert!(result.is_err());
        assert_eq!(calls, 1);
    }
}
