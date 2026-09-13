// standard crates
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::Poll;
use std::time::Duration;

// internal crates
use miru_agent::errors::Error;
use miru_agent::http::with_retry;

// external crates
use futures::poll;
use thiserror::Error as ThisError;
use tokio::time::{advance, Instant};

#[derive(Debug, ThisError)]
#[error("retry test error (network connection: {is_network_conn_err})")]
struct RetryErr {
    is_network_conn_err: bool,
}

impl Error for RetryErr {
    fn is_network_conn_err(&self) -> bool {
        self.is_network_conn_err
    }
}

fn network_err() -> RetryErr {
    RetryErr {
        is_network_conn_err: true,
    }
}

fn app_err() -> RetryErr {
    RetryErr {
        is_network_conn_err: false,
    }
}

/// Lets the timer driver process ready work while this test remains runnable.
/// `advance` need not wake a sleep before its first subsequent poll, and
/// `poll!` returns `Pending` as a value without idling the test. Run the full
/// bound for pending futures so an overdue retry cannot hide at a boundary.
async fn poll_without_advancing<F: Future>(
    mut future: Pin<&mut F>,
    frozen_at: Instant,
) -> Poll<F::Output> {
    for _ in 0..128 {
        tokio::task::yield_now().await;
        assert_eq!(frozen_at, Instant::now(), "polling must not advance time");
        if let Poll::Ready(output) = poll!(future.as_mut()) {
            return Poll::Ready(output);
        }
    }
    Poll::Pending
}

#[tokio::test(start_paused = true)]
async fn success_on_first_attempt() {
    let calls = AtomicUsize::new(0);
    let start = Instant::now();
    let result: Result<&str, RetryErr> = with_retry(|| {
        calls.fetch_add(1, Ordering::SeqCst);
        async { Ok("ok") }
    })
    .await;

    assert_eq!(result.unwrap(), "ok");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(Duration::ZERO, start.elapsed());
}

#[tokio::test(start_paused = true)]
async fn retries_on_network_error_then_succeeds() {
    let calls = AtomicUsize::new(0);
    let start = Instant::now();
    let retry = with_retry(|| {
        let n = calls.fetch_add(1, Ordering::SeqCst);
        async move {
            if n < 2 {
                Err(network_err())
            } else {
                Ok("recovered")
            }
        }
    });
    tokio::pin!(retry);

    assert!(poll!(retry.as_mut()).is_pending());
    assert_eq!(1, calls.load(Ordering::SeqCst));
    assert_eq!(Duration::ZERO, start.elapsed());

    advance(Duration::from_millis(499)).await;
    assert!(
        poll_without_advancing(retry.as_mut(), start + Duration::from_millis(499))
            .await
            .is_pending()
    );
    assert_eq!(1, calls.load(Ordering::SeqCst));
    assert_eq!(Duration::from_millis(499), start.elapsed());

    advance(Duration::from_millis(501)).await;
    assert!(
        poll_without_advancing(retry.as_mut(), start + Duration::from_millis(1000))
            .await
            .is_pending()
    );
    assert_eq!(2, calls.load(Ordering::SeqCst));
    assert_eq!(Duration::from_millis(1000), start.elapsed());

    advance(Duration::from_millis(499)).await;
    assert!(
        poll_without_advancing(retry.as_mut(), start + Duration::from_millis(1499))
            .await
            .is_pending()
    );
    assert_eq!(2, calls.load(Ordering::SeqCst));
    assert_eq!(Duration::from_millis(1499), start.elapsed());

    advance(Duration::from_millis(501)).await;
    let Poll::Ready(result) =
        poll_without_advancing(retry.as_mut(), start + Duration::from_millis(2000)).await
    else {
        panic!("the final retry must complete by 2000 ms");
    };
    assert_eq!("recovered", result.unwrap());
    assert_eq!(calls.load(Ordering::SeqCst), 3, "1 initial + 2 retries");
    assert_eq!(Duration::from_millis(2000), start.elapsed());
}

#[tokio::test(start_paused = true)]
async fn no_retry_on_app_error() {
    let calls = AtomicUsize::new(0);
    let start = Instant::now();
    let result: Result<&str, RetryErr> = with_retry(|| {
        calls.fetch_add(1, Ordering::SeqCst);
        async { Err(app_err()) }
    })
    .await;

    assert!(result.is_err());
    assert!(!result.unwrap_err().is_network_conn_err());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "should not retry app errors"
    );
    assert_eq!(Duration::ZERO, start.elapsed());
}

#[tokio::test(start_paused = true)]
async fn exhausts_retries_on_persistent_network_error() {
    let calls = AtomicUsize::new(0);
    let result: Result<&str, RetryErr> = with_retry(|| {
        calls.fetch_add(1, Ordering::SeqCst);
        async { Err(network_err()) }
    })
    .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().is_network_conn_err());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "should make 3 total attempts (1 initial + 2 retries)"
    );
}

#[tokio::test(start_paused = true)]
async fn network_error_then_app_error_stops_immediately() {
    let calls = AtomicUsize::new(0);
    let start = Instant::now();
    let retry = with_retry(|| {
        let n = calls.fetch_add(1, Ordering::SeqCst);
        async move {
            if n == 0 {
                Err::<&str, RetryErr>(network_err())
            } else {
                Err(app_err())
            }
        }
    });
    tokio::pin!(retry);

    assert!(poll!(retry.as_mut()).is_pending());
    assert_eq!(1, calls.load(Ordering::SeqCst));
    assert_eq!(Duration::ZERO, start.elapsed());

    advance(Duration::from_millis(499)).await;
    assert!(
        poll_without_advancing(retry.as_mut(), start + Duration::from_millis(499))
            .await
            .is_pending()
    );
    assert_eq!(1, calls.load(Ordering::SeqCst));
    assert_eq!(Duration::from_millis(499), start.elapsed());

    advance(Duration::from_millis(501)).await;
    let Poll::Ready(result) =
        poll_without_advancing(retry.as_mut(), start + Duration::from_millis(1000)).await
    else {
        panic!("the application error must complete by 1000 ms");
    };

    assert!(result.is_err());
    assert!(!result.unwrap_err().is_network_conn_err());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "should stop on first non-network error"
    );
    assert_eq!(Duration::from_millis(1000), start.elapsed());
}

#[tokio::test(start_paused = true)]
async fn recovers_on_last_attempt() {
    let calls = AtomicUsize::new(0);
    let result: Result<&str, RetryErr> = with_retry(|| {
        let n = calls.fetch_add(1, Ordering::SeqCst);
        async move {
            if n < 2 {
                Err(network_err())
            } else {
                Ok("last chance")
            }
        }
    })
    .await;

    assert_eq!(result.unwrap(), "last chance");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "should succeed on attempt 3"
    );
}
