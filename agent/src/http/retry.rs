// external crates
use tracing::debug;

const MAX_RETRIES: u32 = 2; // up to 3 total attempts

/// Computes the retry delay with jitter, using subsecond nanos to avoid
/// adding a rand dependency. Not cryptographic, just enough to spread retries.
#[cfg(not(feature = "test"))]
fn retry_delay_ms() -> u64 {
    const BASE_MS: u64 = 500;
    const JITTER_MS: u64 = 500; // delay = base + [0, jitter)
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64;
    BASE_MS + (nanos % JITTER_MS)
}

#[cfg(feature = "test")]
fn retry_delay_ms() -> u64 {
    0
}

/// Retries an async operation up to 3 times on network connection errors.
/// Non-network errors (4xx, 5xx, decode, application) fail immediately.
pub async fn with_retry<F, Fut, T, E>(f: F) -> Result<T, E>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: crate::errors::Error + std::fmt::Display,
{
    let mut attempt = 0;
    loop {
        match f().await {
            Ok(val) => return Ok(val),
            Err(e) => {
                if !e.is_network_conn_err() || attempt >= MAX_RETRIES {
                    return Err(e);
                }
                attempt += 1;
                let delay = retry_delay_ms();
                debug!("network error on attempt {attempt}, retrying in {delay}ms: {e}");
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            }
        }
    }
}
