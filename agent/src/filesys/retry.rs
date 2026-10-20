// standard crates
use std::future::Future;
use std::time::Duration;

// internal crates
use crate::filesys::FileSysErr;

// external crates
use tracing::debug;

/// Runs `op`, and while it fails with an error `should_retry` accepts, sleeps
/// for the next of `delays` and runs it again. Returns the first success, the
/// first error `should_retry` rejects, or the last error once `delays` is
/// used up. Empty `delays` means a single attempt.
pub async fn retry<T, F, Fut>(
    delays: &[Duration],
    should_retry: impl Fn(&FileSysErr) -> bool,
    mut op: F,
) -> Result<T, FileSysErr>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, FileSysErr>>,
{
    let mut delays = delays.iter();
    loop {
        match op().await {
            Err(err) if should_retry(&err) => {
                let Some(delay) = delays.next() else {
                    return Err(err);
                };
                debug!("file operation failed, retrying in {delay:?}: {err}");
                tokio::time::sleep(*delay).await;
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
    use crate::filesys::errors::{DeleteFileErr, PathDoesNotExistErr};
    use crate::filesys::File;
    use crate::trace;

    const DELAYS: [Duration; 2] = [Duration::from_millis(1), Duration::from_millis(1)];

    fn transient_err() -> FileSysErr {
        FileSysErr::DeleteFileErr(DeleteFileErr {
            source: Box::new(std::io::Error::other("sharing violation")),
            file: File::new("device-api.json"),
            trace: trace!(),
        })
    }

    /// Runs `retry` over an op that fails `failures` times with `err`, and
    /// returns the result and how many times the op ran.
    async fn run(
        delays: &[Duration],
        failures: u32,
        err: fn() -> FileSysErr,
    ) -> (Result<(), FileSysErr>, u32) {
        let calls = AtomicU32::new(0);
        let result = retry(
            delays,
            |e| !matches!(e, FileSysErr::PathDoesNotExistErr(_)),
            || async {
                if calls.fetch_add(1, Ordering::SeqCst) < failures {
                    Err(err())
                } else {
                    Ok(())
                }
            },
        )
        .await;
        (result, calls.load(Ordering::SeqCst))
    }

    #[tokio::test]
    async fn succeeds_first_try_without_retrying() {
        let (result, calls) = run(&DELAYS, 0, transient_err).await;
        assert!(result.is_ok());
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn retries_until_success() {
        let (result, calls) = run(&DELAYS, 2, transient_err).await;
        assert!(result.is_ok());
        assert_eq!(calls, 3);
    }

    #[tokio::test]
    async fn returns_last_error_when_delays_run_out() {
        let (result, calls) = run(&DELAYS, 5, transient_err).await;
        assert!(matches!(result, Err(FileSysErr::DeleteFileErr(_))));
        assert_eq!(calls, 3);
    }

    #[tokio::test]
    async fn no_delays_means_a_single_attempt() {
        let (result, calls) = run(&[], 1, transient_err).await;
        assert!(result.is_err());
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn rejected_error_is_returned_without_retrying() {
        let missing = || {
            FileSysErr::PathDoesNotExistErr(PathDoesNotExistErr {
                path: std::path::PathBuf::from("missing.json"),
                trace: trace!(),
            })
        };
        let (result, calls) = run(&DELAYS, 1, missing).await;
        assert!(matches!(result, Err(FileSysErr::PathDoesNotExistErr(_))));
        assert_eq!(calls, 1);
    }
}
