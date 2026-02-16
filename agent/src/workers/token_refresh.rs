// standard library
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

// internal crates
use crate::authn::token_mngr::TokenManagerExt;
use crate::cooldown;
use crate::errors::*;

// external crates
use chrono::Utc;
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
        let next_wait = match token_mngr.refresh_token().await {
            Ok(_) => {
                if err_streak > 0 {
                    info!(
                        "token refreshed successfully after an error streak of {err_streak} errors"
                    );
                } else {
                    info!("token refreshed successfully");
                }
                err_streak = 0;
                calc_refresh_wait(
                    token_mngr,
                    options.refresh_advance_secs,
                    err_streak,
                    options.backoff,
                )
                .await
            }
            Err(e) => {
                if e.is_network_connection_error() {
                    debug!("unable to refresh token due to a network connection error: {e:?}");
                    calc_refresh_wait(
                        token_mngr,
                        options.refresh_advance_secs,
                        // we want to try network connection errors again immediately
                        // (even if the previous errors were not network connection
                        // errors) so we use an error streak of 0
                        0,
                        options.backoff,
                    )
                    .await
                } else {
                    error!("error refreshing token (error streak: {err_streak}): {e:?}");
                    err_streak += 1;
                    calc_refresh_wait(
                        token_mngr,
                        options.refresh_advance_secs,
                        err_streak,
                        options.backoff,
                    )
                    .await
                }
            }
        };

        let refresh_time = Utc::now() + next_wait;
        debug!("waiting until {:?} to refresh token", refresh_time);

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

pub async fn calc_refresh_wait<TokenManagerT: TokenManagerExt>(
    token_mngr: &TokenManagerT,
    refresh_advance_secs: i64,
    err_streak: u32,
    backoff: cooldown::Backoff,
) -> Duration {
    // calculate the cooldown period
    let cooldown_secs = cooldown::calc(&backoff, err_streak);

    match token_mngr.get_token().await {
        Ok(token) => {
            let expiration = token.expires_at;
            let secs_until_exp = (expiration - Utc::now()).num_seconds();

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
    }
}
