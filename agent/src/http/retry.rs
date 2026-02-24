use tracing::debug;

const MAX_RETRIES: u32 = 2; // up to 3 total attempts

#[cfg(not(feature = "test"))]
const RETRY_BASE_MS: u64 = 500;
#[cfg(not(feature = "test"))]
const RETRY_JITTER_MS: u64 = 500; // delay = base + [0, jitter)
#[cfg(feature = "test")]
const RETRY_BASE_MS: u64 = 0;
#[cfg(feature = "test")]
const RETRY_JITTER_MS: u64 = 0;

/// Computes the retry delay with jitter, using subsecond nanos to avoid
/// adding a rand dependency. Not cryptographic, just enough to spread retries.
fn retry_delay_ms() -> u64 {
    if RETRY_JITTER_MS == 0 {
        return RETRY_BASE_MS;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64;
    RETRY_BASE_MS + (nanos % RETRY_JITTER_MS)
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
