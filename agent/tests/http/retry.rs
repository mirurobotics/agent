// standard crates
use std::cell::RefCell;
use std::time::Duration;

// internal crates
use miru_agent::errors::Error;
use miru_agent::http::with_retry;

// external crates
use thiserror::Error as ThisError;
use tokio::time::Instant;

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

#[tokio::test(start_paused = true)]
async fn success_on_first_attempt() {
    let attempts = RefCell::new(Vec::new());
    let start = Instant::now();
    let result: Result<&str, RetryErr> = with_retry(|| {
        attempts.borrow_mut().push(Instant::now());
        async { Ok("ok") }
    })
    .await;

    assert_eq!("ok", result.unwrap());
    let attempts = attempts.into_inner();
    assert_eq!(vec![start], attempts);
    assert_eq!(start, Instant::now());
}

#[tokio::test(start_paused = true)]
async fn retries_on_network_error_then_succeeds() {
    let attempts = RefCell::new(Vec::new());
    let start = Instant::now();
    let result = with_retry(|| {
        let n = attempts.borrow().len();
        attempts.borrow_mut().push(Instant::now());
        async move {
            if n < 2 {
                Err(network_err())
            } else {
                Ok("recovered")
            }
        }
    })
    .await;

    assert_eq!("recovered", result.unwrap());
    let attempts = attempts.into_inner();
    assert_eq!(3, attempts.len());
    assert_eq!(start, attempts[0]);
    for pair in attempts.windows(2) {
        let gap = pair[1] - pair[0];
        assert!((Duration::from_millis(500)..=Duration::from_millis(1000)).contains(&gap));
    }
    assert_eq!(*attempts.last().unwrap(), Instant::now());
}

#[tokio::test(start_paused = true)]
async fn no_retry_on_app_error() {
    let attempts = RefCell::new(Vec::new());
    let start = Instant::now();
    let result: Result<&str, RetryErr> = with_retry(|| {
        attempts.borrow_mut().push(Instant::now());
        async { Err(app_err()) }
    })
    .await;

    assert!(result.is_err());
    assert!(!result.unwrap_err().is_network_conn_err());
    let attempts = attempts.into_inner();
    assert_eq!(vec![start], attempts);
    assert_eq!(start, Instant::now());
}

#[tokio::test(start_paused = true)]
async fn exhausts_retries_on_persistent_network_error() {
    let attempts = RefCell::new(Vec::new());
    let start = Instant::now();
    let result: Result<&str, RetryErr> = with_retry(|| {
        attempts.borrow_mut().push(Instant::now());
        async { Err(network_err()) }
    })
    .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().is_network_conn_err());
    let attempts = attempts.into_inner();
    assert_eq!(3, attempts.len());
    assert_eq!(start, attempts[0]);
    for pair in attempts.windows(2) {
        let gap = pair[1] - pair[0];
        assert!((Duration::from_millis(500)..=Duration::from_millis(1000)).contains(&gap));
    }
    assert_eq!(*attempts.last().unwrap(), Instant::now());
}

#[tokio::test(start_paused = true)]
async fn network_error_then_app_error_stops_immediately() {
    let attempts = RefCell::new(Vec::new());
    let start = Instant::now();
    let result = with_retry(|| {
        let n = attempts.borrow().len();
        attempts.borrow_mut().push(Instant::now());
        async move {
            if n == 0 {
                Err::<&str, RetryErr>(network_err())
            } else {
                Err(app_err())
            }
        }
    })
    .await;

    assert!(result.is_err());
    assert!(!result.unwrap_err().is_network_conn_err());
    let attempts = attempts.into_inner();
    assert_eq!(2, attempts.len());
    assert_eq!(start, attempts[0]);
    for pair in attempts.windows(2) {
        let gap = pair[1] - pair[0];
        assert!((Duration::from_millis(500)..=Duration::from_millis(1000)).contains(&gap));
    }
    assert_eq!(*attempts.last().unwrap(), Instant::now());
}

#[tokio::test(start_paused = true)]
async fn recovers_on_last_attempt() {
    let attempts = RefCell::new(Vec::new());
    let start = Instant::now();
    let result: Result<&str, RetryErr> = with_retry(|| {
        let n = attempts.borrow().len();
        attempts.borrow_mut().push(Instant::now());
        async move {
            if n < 2 {
                Err(network_err())
            } else {
                Ok("last chance")
            }
        }
    })
    .await;

    assert_eq!("last chance", result.unwrap());
    let attempts = attempts.into_inner();
    assert_eq!(3, attempts.len());
    assert_eq!(start, attempts[0]);
    for pair in attempts.windows(2) {
        let gap = pair[1] - pair[0];
        assert!((Duration::from_millis(500)..=Duration::from_millis(1000)).contains(&gap));
    }
    assert_eq!(*attempts.last().unwrap(), Instant::now());
}
