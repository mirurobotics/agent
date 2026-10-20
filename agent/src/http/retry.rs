// standard crates
use std::time::Duration;

// internal crates
use crate::cooldown;

const MAX_RETRIES: u32 = 2; // up to 3 total attempts

/// Computes the retry delay with jitter, using subsecond nanos to avoid
/// adding a rand dependency. Not cryptographic, just enough to spread retries.
fn retry_delay_ms() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64;
    retry_delay_from_nanos(nanos)
}

fn retry_delay_from_nanos(nanos: u64) -> u64 {
    const BASE_MS: u64 = 500;
    const JITTER_MS: u64 = 500; // delay = base + [0, jitter)
    BASE_MS + (nanos % JITTER_MS)
}

/// Retries an async operation up to 3 times on network connection errors.
/// Non-network errors (4xx, 5xx, decode, application) fail immediately.
pub async fn with_retry<F, Fut, T, E>(f: F) -> Result<T, E>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: crate::errors::Error + std::fmt::Display,
{
    cooldown::retry(
        |n| (n < MAX_RETRIES).then(|| Duration::from_millis(retry_delay_ms())),
        |e: &E| e.is_network_conn_err(),
        tokio::time::sleep,
        f,
    )
    .await
}

#[cfg(test)]
mod tests {
    // internal crates
    use super::retry_delay_from_nanos;

    #[test]
    fn retry_delay_from_nanos_respects_jitter_boundaries() {
        for (nanos, expected_ms) in [(0, 500), (499, 999), (500, 500)] {
            assert_eq!(
                expected_ms,
                retry_delay_from_nanos(nanos),
                "subsecond nanoseconds: {nanos}"
            );
        }
    }
}
