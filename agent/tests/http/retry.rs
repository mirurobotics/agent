use std::sync::atomic::{AtomicUsize, Ordering};

use miru_agent::errors::Error;
use miru_agent::http::errors::{HTTPErr, MockErr};
use miru_agent::http::with_retry;

fn network_err() -> HTTPErr {
    HTTPErr::MockErr(MockErr {
        is_network_conn_err: true,
    })
}

fn app_err() -> HTTPErr {
    HTTPErr::MockErr(MockErr {
        is_network_conn_err: false,
    })
}

#[tokio::test]
async fn success_on_first_attempt() {
    let calls = AtomicUsize::new(0);
    let result: Result<&str, HTTPErr> = with_retry(|| {
        calls.fetch_add(1, Ordering::SeqCst);
        async { Ok("ok") }
    })
    .await;

    assert_eq!(result.unwrap(), "ok");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn retries_on_network_error_then_succeeds() {
    let calls = AtomicUsize::new(0);
    let result: Result<&str, HTTPErr> = with_retry(|| {
        let n = calls.fetch_add(1, Ordering::SeqCst);
        async move {
            if n < 2 {
                Err(network_err())
            } else {
                Ok("recovered")
            }
        }
    })
    .await;

    assert_eq!(result.unwrap(), "recovered");
    assert_eq!(calls.load(Ordering::SeqCst), 3, "1 initial + 2 retries");
}

#[tokio::test]
async fn no_retry_on_app_error() {
    let calls = AtomicUsize::new(0);
    let result: Result<&str, HTTPErr> = with_retry(|| {
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
}

#[tokio::test]
async fn exhausts_retries_on_persistent_network_error() {
    let calls = AtomicUsize::new(0);
    let result: Result<&str, HTTPErr> = with_retry(|| {
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

#[tokio::test]
async fn network_error_then_app_error_stops_immediately() {
    let calls = AtomicUsize::new(0);
    let result: Result<&str, HTTPErr> = with_retry(|| {
        let n = calls.fetch_add(1, Ordering::SeqCst);
        async move {
            if n == 0 {
                Err(network_err())
            } else {
                Err(app_err())
            }
        }
    })
    .await;

    assert!(result.is_err());
    assert!(!result.unwrap_err().is_network_conn_err());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "should stop on first non-network error"
    );
}

#[tokio::test]
async fn recovers_on_last_attempt() {
    let calls = AtomicUsize::new(0);
    let result: Result<&str, HTTPErr> = with_retry(|| {
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
