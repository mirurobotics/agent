// standard crates
use std::fmt::Display;
use std::future::Future;
use std::time::Duration;

// external crates
use tracing::debug;

/// Runs `op`, and while it fails with an error `should_retry` accepts, asks
/// `delay` how long to wait before the next try, waits with `sleep`, and runs
/// it again. `delay` gets the retry number (0 for the first retry) and returns
/// `None` to stop. Returns the first success, the first error `should_retry`
/// rejects, or the last error once `delay` returns `None`.
///
/// For a fixed list of waits, pass `|n| DELAYS.get(n as usize).copied()`.
pub async fn retry<T, E, F, Fut, S, SFut>(
    delay: impl Fn(u32) -> Option<Duration>,
    should_retry: impl Fn(&E) -> bool,
    sleep: S,
    mut op: F,
) -> Result<T, E>
where
    E: Display,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    S: Fn(Duration) -> SFut,
    SFut: Future<Output = ()>,
{
    let mut retries: u32 = 0;
    loop {
        match op().await {
            Err(err) if should_retry(&err) => {
                let Some(wait) = delay(retries) else {
                    return Err(err);
                };
                debug!("operation failed, retrying in {wait:?}: {err}");
                sleep(wait).await;
                retries = retries.saturating_add(1);
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

    const DELAYS: [Duration; 2] = [Duration::from_millis(1), Duration::from_millis(2)];

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
            |n| delays.get(n as usize).copied(),
            |e: &&str| *e != "fatal",
            tokio::time::sleep,
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
    async fn waits_for_each_delay_in_order() {
        let waits = std::sync::Mutex::new(Vec::new());
        let result: Result<(), &str> = retry(
            |n| DELAYS.get(n as usize).copied(),
            |_| true,
            |wait| {
                waits.lock().unwrap().push(wait);
                std::future::ready(())
            },
            || async { Err("transient") },
        )
        .await;
        assert!(result.is_err());
        assert_eq!(*waits.lock().unwrap(), DELAYS.to_vec());
    }

    #[tokio::test]
    async fn rejected_error_is_returned_without_retrying() {
        let (result, calls) = run(&DELAYS, 1, "fatal").await;
        assert_eq!(result, Err("fatal"));
        assert_eq!(calls, 1);
    }
}
