// standard crates
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

// internal crates
use crate::authn::TokenManagerExt;
use crate::clock::Clock;
use crate::cooldown;
use crate::errors::*;

// external crates
use tracing::{debug, error, info};

#[derive(Debug, Clone)]
pub struct TokenRefreshWorkerOptions {
    pub refresh_advance_secs: i64,
    pub backoff: cooldown::Backoff,
}

impl Default for TokenRefreshWorkerOptions {
    fn default() -> Self {
        Self {
            refresh_advance_secs: 60 * 15, // 15 minutes
            backoff: cooldown::Backoff {
                base_secs: 12,
                growth_factor: 2,
                max_secs: 60 * 60, // 1 hour
            },
        }
    }
}

pub async fn run_token_refresh_worker<F, Fut, TokenManagerT: TokenManagerExt>(
    options: &TokenRefreshWorkerOptions,
    token_mngr: &TokenManagerT,
    clock: &Clock,
    sleep_fn: F, // for testing purposes
    mut shutdown_signal: Pin<Box<impl Future<Output = ()> + Send + 'static>>,
) where
    F: Fn(Duration) -> Fut,
    Fut: Future<Output = ()> + Send,
{
    info!("Running token refresh worker");
    let mut err_streak = 0;

    loop {
        // refresh
        let next_wait = refresh_once(options, token_mngr, clock, &mut err_streak).await;
        debug!("waiting {}s to refresh token", next_wait.as_secs());

        // wait to refresh or shutdown if the signal is received
        tokio::select! {
            _ = shutdown_signal.as_mut() => {
                info!("token refresh worker shutdown complete");
                return;
            }
            _ = sleep_fn(next_wait) => {},
        }
    }
}

/// Attempt a single token refresh, update the error streak, and return the
/// wait until the next refresh attempt.
async fn refresh_once<TokenManagerT: TokenManagerExt>(
    options: &TokenRefreshWorkerOptions,
    token_mngr: &TokenManagerT,
    clock: &Clock,
    err_streak: &mut u32,
) -> Duration {
    match token_mngr.refresh_token().await {
        Ok(_) => {
            if *err_streak > 0 {
                info!(
                    "token refreshed successfully after an error streak of {} errors",
                    *err_streak
                );
            } else {
                info!("token refreshed successfully");
            }
            *err_streak = 0;
            calc_refresh_wait(
                token_mngr,
                clock,
                options.refresh_advance_secs,
                *err_streak,
                options.backoff,
            )
            .await
        }
        Err(e) if e.is_network_conn_err() => {
            debug!("unable to refresh token due to a network connection error: {e:?}");
            calc_refresh_wait(
                token_mngr,
                clock,
                options.refresh_advance_secs,
                // we want to try network connection errors again immediately
                // (even if the previous errors were not network connection
                // errors) so we use an error streak of 0
                0,
                options.backoff,
            )
            .await
        }
        Err(e) => {
            error!(
                "error refreshing token (error streak: {}): {e:?}",
                *err_streak
            );
            *err_streak += 1;
            calc_refresh_wait(
                token_mngr,
                clock,
                options.refresh_advance_secs,
                *err_streak,
                options.backoff,
            )
            .await
        }
    }
}

/// The wait until the next refresh. The token's expiry is the server's time,
/// so it is compared against the server's time when known. Before the first
/// backend response only the system clock is available, and it may be decades
/// off (a robot that booted at 1970 would read a fresh token as valid for ~56
/// years), so the wait is capped at the backoff maximum until the backend's
/// time is known.
pub async fn calc_refresh_wait<TokenManagerT: TokenManagerExt>(
    token_mngr: &TokenManagerT,
    clock: &Clock,
    refresh_advance_secs: i64,
    err_streak: u32,
    backoff: cooldown::Backoff,
) -> Duration {
    // calculate the cooldown period
    let cooldown_secs = cooldown::calc(&backoff, err_streak);

    let (now, from_server) = match clock.server_now() {
        Some(now) => (now, true),
        None => (clock.system_now(), false),
    };
    let wait = match token_mngr.get_token().await {
        Ok(token) => {
            let expiration = token.expires_at;
            let secs_until_exp = (expiration - now).num_seconds();

            // if the token will expire within our refresh advance period, only wait
            // for the cooldown period before refreshing the token
            if secs_until_exp < refresh_advance_secs {
                Duration::from_secs(cooldown_secs as u64)

            // if the token expires after our refresh advance period, wait until the
            // refresh advance period begins to refresh the token
            } else {
                Duration::from_secs((secs_until_exp - refresh_advance_secs) as u64)
            }
        }
        Err(e) => {
            error!("Error fetching token from token manager: {:#?}", e);
            Duration::from_secs(cooldown_secs as u64)
        }
    };
    if from_server {
        wait
    } else {
        wait.min(Duration::from_secs(backoff.max_secs.max(0) as u64))
    }
}
