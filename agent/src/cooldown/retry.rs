// standard crates
use std::fmt::Display;
use std::future::Future;
use std::time::Duration;

// external crates
use tracing::debug;

/// Runs `op`, and while it fails with an error `should_retry` accepts, sleeps
/// for the next of `delays` and runs it again. Returns the first success, the
/// first error `should_retry` rejects, or the last error once `delays` is
/// used up. Empty `delays` means a single attempt.
pub async fn retry<T, E, F, Fut>(
    delays: &[Duration],
    should_retry: impl Fn(&E) -> bool,
    mut op: F,
) -> Result<T, E>
where
    E: Display,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let mut delays = delays.iter();
    loop {
        match op().await {
            Err(err) if should_retry(&err) => {
                let Some(delay) = delays.next() else {
                    return Err(err);
                };
                debug!("operation failed, retrying in {delay:?}: {err}");
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

    const DELAYS: [Duration; 2] = [Duration::from_millis(1), Duration::from_millis(1)];

    /// Runs `retry` over an op that fails `failures` times with `err`, retrying
    /// every error except `"fatal"`, and returns the result and how many times
    /// the op ran.
    async fn run(
        delays: &[Duration],
        failures: u32,
        err: &'static str,
    ) -> (Result<(), &'static str>, u32) {
        let calls = AtomicU32::new(0);
        let result = retry(
            delays,
            |e: &&str| *e != "fatal",
            || async {
                if calls.fetch_add(1, Ordering::SeqCst) < failures {
                    Err(err)
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
        let (result, calls) = run(&DELAYS, 0, "transient").await;
        assert!(result.is_ok());
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn retries_until_success() {
        let (result, calls) = run(&DELAYS, 2, "transient").await;
        assert!(result.is_ok());
        assert_eq!(calls, 3);
    }

    #[tokio::test]
    async fn returns_last_error_when_delays_run_out() {
        let (result, calls) = run(&DELAYS, 5, "transient").await;
        assert_eq!(result, Err("transient"));
        assert_eq!(calls, 3);
    }

    #[tokio::test]
    async fn no_delays_means_a_single_attempt() {
        let (result, calls) = run(&[], 1, "transient").await;
        assert!(result.is_err());
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn rejected_error_is_returned_without_retrying() {
        let (result, calls) = run(&DELAYS, 1, "fatal").await;
        assert_eq!(result, Err("fatal"));
        assert_eq!(calls, 1);
    }
}
