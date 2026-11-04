// standard crates
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::mocks::{
    error::SleepController,
    token_manager::{MockTokenManager, TokenManagerCall},
};
use miru_agent::authn::errors::MockError;
use miru_agent::authn::{AuthnErr, Token};
use miru_agent::clock::Clock;
use miru_agent::cooldown;
use miru_agent::trace;
use miru_agent::workers::token_refresh::{
    calc_refresh_wait, run_token_refresh_worker, TokenRefreshWorkerOptions,
};

// external crates
use chrono::{DateTime, TimeDelta, Utc};

/// A clock that has heard from a backend whose time matches the system clock.
fn synced_clock() -> Clock {
    let clock = Clock::new();
    clock.observe_server_time(Utc::now());
    clock
}

/// A clock whose system clock reads `system`, with the server's time unknown.
fn system_clock_at(system: DateTime<Utc>) -> Clock {
    Clock::from_sources(Arc::new(move || system), Arc::new(std::time::Instant::now))
}

pub mod run_refresh_token_worker {
    use super::*;

    #[tokio::test]
    async fn success() {
        // create the token manager
        let token = Token {
            token: "token".to_string(),
            expires_at: Utc::now(),
        };
        let token_mngr = Arc::new(MockTokenManager::new(token));

        // create a controllable sleep function
        let sleep_ctrl = Arc::new(SleepController::new());

        // create the shutdown signal
        let (shutdown_tx, _shutdown_rx): (tokio::sync::broadcast::Sender<()>, _) =
            tokio::sync::broadcast::channel(1);
        let mut shutdown_rx = shutdown_tx.subscribe();
        let shutdown_signal = async move {
            let _ = shutdown_rx.recv().await;
        };

        // set the cooldown options
        let refresh_advance_secs = 10 * 60; // 10 minutes
        let cooldown = cooldown::Backoff {
            base_secs: 30,
            growth_factor: 2,
            max_secs: 12 * 60 * 60,
        };

        // run the worker
        let token_mngr_for_spawn = token_mngr.clone();
        let sleep_ctrl_for_spawn = sleep_ctrl.clone();
        let options = TokenRefreshWorkerOptions {
            refresh_advance_secs,
            backoff: cooldown,
        };
        let token_refresh_handle = tokio::spawn(async move {
            run_token_refresh_worker(
                &options,
                token_mngr_for_spawn.as_ref(),
                &synced_clock(),
                sleep_ctrl_for_spawn.sleep_fn(),
                Box::pin(shutdown_signal),
            )
            .await;
        });

        // these sleeps should wait for the number of base secs since the token is
        // expired
        let mut expected_get_token_calls = 0;
        let mut expected_refresh_token_calls = 0;
        for _ in 0..10 {
            sleep_ctrl.await_sleep().await;
            let last_sleep = sleep_ctrl.get_last_attempted_sleep().unwrap();
            assert_eq!(last_sleep.as_secs(), cooldown.base_secs as u64);
            expected_get_token_calls += 1;
            expected_refresh_token_calls += 1;
            assert_eq!(token_mngr.num_get_token_calls(), expected_get_token_calls);
            assert_eq!(
                token_mngr.num_refresh_token_calls(),
                expected_refresh_token_calls
            );
            assert_eq!(
                token_mngr.get_calls(),
                [TokenManagerCall::RefreshToken, TokenManagerCall::GetToken]
                    .repeat(expected_get_token_calls)
            );
            sleep_ctrl.release().await;
        }

        // these sleeps should wait until token exp - refresh_advance_secs since we set
        // the token to expire in 100 minutes
        let token_exp_duration = TimeDelta::minutes(100);
        token_mngr.set_token(Token {
            token: "token".to_string(),
            expires_at: Utc::now() + token_exp_duration,
        });
        sleep_ctrl.release().await;
        for _ in 0..10 {
            sleep_ctrl.await_sleep().await;
            let last_sleep = sleep_ctrl.get_last_attempted_sleep().unwrap();
            let expected_sleep_secs = token_exp_duration.num_seconds() - refresh_advance_secs;
            assert!(last_sleep.as_secs() <= expected_sleep_secs as u64);
            assert!(last_sleep.as_secs() >= expected_sleep_secs as u64 - 2);
            expected_get_token_calls += 1;
            expected_refresh_token_calls += 1;
            assert_eq!(token_mngr.num_get_token_calls(), expected_get_token_calls);
            assert_eq!(
                token_mngr.num_refresh_token_calls(),
                expected_refresh_token_calls
            );
            sleep_ctrl.release().await;
        }

        // shutdown the token manager and refresh loop
        shutdown_tx.send(()).unwrap();
        token_refresh_handle.await.unwrap();
    }

    #[tokio::test]
    async fn network_error() {
        // create the token manager
        let token = Token {
            token: "token".to_string(),
            expires_at: Utc::now(),
        };
        let token_mngr = MockTokenManager::new(token);
        token_mngr.set_refresh_token(Box::new(|| {
            Err(AuthnErr::MockError(MockError {
                is_network_conn_err: true,
                trace: trace!(),
            }))
        }));
        let token_mngr = Arc::new(token_mngr);

        // create a controllable sleep function
        let sleep_ctrl = Arc::new(SleepController::new());

        // create the shutdown signal
        let (shutdown_tx, _shutdown_rx): (tokio::sync::broadcast::Sender<()>, _) =
            tokio::sync::broadcast::channel(1);
        let mut shutdown_rx = shutdown_tx.subscribe();
        let shutdown_signal = async move {
            let _ = shutdown_rx.recv().await;
        };

        // set the cooldown options
        let refresh_advance_secs = 10 * 60; // 10 minutes
        let cooldown = cooldown::Backoff {
            base_secs: 30,
            growth_factor: 2,
            max_secs: 12 * 60 * 60,
        };

        // run the worker
        let token_mngr_for_spawn = token_mngr.clone();
        let sleep_ctrl_for_spawn = sleep_ctrl.clone();
        let options = TokenRefreshWorkerOptions {
            refresh_advance_secs,
            backoff: cooldown,
        };
        let token_refresh_handle = tokio::spawn(async move {
            run_token_refresh_worker(
                &options,
                token_mngr_for_spawn.as_ref(),
                &synced_clock(),
                sleep_ctrl_for_spawn.sleep_fn(),
                Box::pin(shutdown_signal),
            )
            .await;
        });

        // all sleeps should wait the base number of seconds
        let mut expected_get_token_calls = 0;
        let mut expected_refresh_token_calls = 0;
        for _ in 0..10 {
            sleep_ctrl.release().await;
            sleep_ctrl.await_sleep().await;
            let last_sleep = sleep_ctrl.get_last_attempted_sleep().unwrap();
            let expected_sleep_secs = cooldown.base_secs;
            assert_eq!(last_sleep.as_secs(), expected_sleep_secs as u64);
            expected_get_token_calls += 1;
            expected_refresh_token_calls += 1;
            assert_eq!(token_mngr.num_get_token_calls(), expected_get_token_calls);
            assert_eq!(
                token_mngr.num_refresh_token_calls(),
                expected_refresh_token_calls
            );
        }

        // shutdown the token manager and refresh loop
        shutdown_tx.send(()).unwrap();
        token_refresh_handle.await.unwrap();
    }

    #[tokio::test]
    async fn non_network_error() {
        // create the token manager
        let token = Token {
            token: "token".to_string(),
            expires_at: Utc::now(),
        };
        let token_mngr = MockTokenManager::new(token);
        token_mngr.set_refresh_token(Box::new(|| {
            Err(AuthnErr::MockError(MockError {
                is_network_conn_err: false,
                trace: trace!(),
            }))
        }));
        let token_mngr = Arc::new(token_mngr);

        // create a controllable sleep function
        let sleep_ctrl = Arc::new(SleepController::new());

        // create the shutdown signal
        let (shutdown_tx, _shutdown_rx): (tokio::sync::broadcast::Sender<()>, _) =
            tokio::sync::broadcast::channel(1);
        let mut shutdown_rx = shutdown_tx.subscribe();
        let shutdown_signal = async move {
            let _ = shutdown_rx.recv().await;
        };

        // set the cooldown options
        let refresh_advance_secs = 10 * 60; // 10 minutes
        let cooldown = cooldown::Backoff {
            base_secs: 30,
            growth_factor: 2,
            max_secs: 12 * 60 * 60,
        };

        // run the worker
        let token_mngr_for_spawn = token_mngr.clone();
        let sleep_ctrl_for_spawn = sleep_ctrl.clone();
        let options = TokenRefreshWorkerOptions {
            refresh_advance_secs,
            backoff: cooldown,
        };
        let token_refresh_handle = tokio::spawn(async move {
            run_token_refresh_worker(
                &options,
                token_mngr_for_spawn.as_ref(),
                &synced_clock(),
                sleep_ctrl_for_spawn.sleep_fn(),
                Box::pin(shutdown_signal),
            )
            .await;
        });

        // sleeps should wait according to the exp backoff
        let mut expected_get_token_calls = 0;
        let mut expected_refresh_token_calls = 0;
        for i in 0..10 {
            sleep_ctrl.release().await;
            sleep_ctrl.await_sleep().await;
            let last_sleep = sleep_ctrl.get_last_attempted_sleep().unwrap();
            let expected_sleep_secs = cooldown::calc(&cooldown, i + 1);
            assert_eq!(last_sleep.as_secs(), expected_sleep_secs as u64);
            expected_get_token_calls += 1;
            expected_refresh_token_calls += 1;
            assert_eq!(token_mngr.num_get_token_calls(), expected_get_token_calls);
            assert_eq!(
                token_mngr.num_refresh_token_calls(),
                expected_refresh_token_calls
            );
        }

        // shutdown the token manager and refresh loop
        shutdown_tx.send(()).unwrap();
        token_refresh_handle.await.unwrap();
    }

    #[tokio::test]
    async fn error_recovery() {
        // create the token manager
        let token = Token {
            token: "token".to_string(),
            expires_at: Utc::now(),
        };
        let token_mngr = Arc::new(MockTokenManager::new(token));
        token_mngr.set_refresh_token(Box::new(|| {
            Err(AuthnErr::MockError(MockError {
                is_network_conn_err: false,
                trace: trace!(),
            }))
        }));

        // create a controllable sleep function
        let sleep_ctrl = Arc::new(SleepController::new());

        // create the shutdown signal
        let (shutdown_tx, _shutdown_rx): (tokio::sync::broadcast::Sender<()>, _) =
            tokio::sync::broadcast::channel(1);
        let mut shutdown_rx = shutdown_tx.subscribe();
        let shutdown_signal = async move {
            let _ = shutdown_rx.recv().await;
        };

        // set the cooldown options
        let refresh_advance_secs = 10 * 60; // 10 minutes
        let cooldown = cooldown::Backoff {
            base_secs: 30,
            growth_factor: 2,
            max_secs: 12 * 60 * 60,
        };

        // run the worker
        let token_mngr_for_spawn = token_mngr.clone();
        let sleep_ctrl_for_spawn = sleep_ctrl.clone();
        let options = TokenRefreshWorkerOptions {
            refresh_advance_secs,
            backoff: cooldown,
        };
        let token_refresh_handle = tokio::spawn(async move {
            run_token_refresh_worker(
                &options,
                token_mngr_for_spawn.as_ref(),
                &synced_clock(),
                sleep_ctrl_for_spawn.sleep_fn(),
                Box::pin(shutdown_signal),
            )
            .await;
        });

        // sleeps should wait according to the exp backoff
        let mut expected_get_token_calls = 0;
        let mut expected_refresh_token_calls = 0;
        for i in 0..10 {
            sleep_ctrl.release().await;
            sleep_ctrl.await_sleep().await;
            let last_sleep = sleep_ctrl.get_last_attempted_sleep().unwrap();
            let expected_sleep_secs = cooldown::calc(&cooldown, i + 1);
            assert_eq!(last_sleep.as_secs(), expected_sleep_secs as u64);
            expected_get_token_calls += 1;
            expected_refresh_token_calls += 1;
            assert_eq!(token_mngr.num_get_token_calls(), expected_get_token_calls);
            assert_eq!(
                token_mngr.num_refresh_token_calls(),
                expected_refresh_token_calls
            );
        }

        token_mngr.set_refresh_token(Box::new(|| Ok(())));

        // all sleeps should wait the base number of seconds after recovery
        sleep_ctrl.release().await;
        for _ in 0..10 {
            sleep_ctrl.release().await;
            sleep_ctrl.await_sleep().await;
            let last_sleep = sleep_ctrl.get_last_attempted_sleep().unwrap();
            let expected_sleep_secs = cooldown.base_secs;
            assert_eq!(last_sleep.as_secs(), expected_sleep_secs as u64);
            expected_get_token_calls += 1;
            expected_refresh_token_calls += 1;
            assert_eq!(token_mngr.num_get_token_calls(), expected_get_token_calls);
            assert_eq!(
                token_mngr.num_refresh_token_calls(),
                expected_refresh_token_calls
            );
        }

        // shutdown the token manager and refresh loop
        shutdown_tx.send(()).unwrap();
        token_refresh_handle.await.unwrap();
    }
}

pub mod calc_refresh_wait {
    use super::*;

    #[tokio::test]
    async fn expired_in_past() {
        let token = Token {
            token: "token".to_string(),
            expires_at: Utc::now() - TimeDelta::minutes(60),
        };
        let token_mngr = MockTokenManager::new(token);

        let refresh_advance = 10 * 60; // 10 minutes
        let cooldown = cooldown::Backoff {
            base_secs: 30,
            growth_factor: 2,
            max_secs: 12 * 60 * 60,
        };

        for i in 0..10 {
            let err_streak = i;
            let actual = calc_refresh_wait(
                &token_mngr,
                &synced_clock(),
                refresh_advance,
                err_streak,
                cooldown,
            )
            .await;
            let expected_secs = cooldown::calc(&cooldown, err_streak);
            let expected = Duration::from_secs(expected_secs as u64);
            assert_eq!(expected, actual);
        }
    }

    #[tokio::test]
    async fn expires_less_than_10_minutes() {
        let token = Token {
            token: "token".to_string(),
            expires_at: Utc::now() + TimeDelta::minutes(9),
        };
        let token_mngr = MockTokenManager::new(token);

        let refresh_advance = 10 * 60; // 10 minutes
        let cooldown = cooldown::Backoff {
            base_secs: 30,
            growth_factor: 2,
            max_secs: 12 * 60 * 60,
        };

        for i in 0..10 {
            let err_streak = i;
            let sleep_duration = calc_refresh_wait(
                &token_mngr,
                &synced_clock(),
                refresh_advance,
                err_streak,
                cooldown,
            )
            .await;
            let expected_secs = cooldown::calc(&cooldown, err_streak);
            let expected = Duration::from_secs(expected_secs as u64);
            assert_eq!(sleep_duration, expected);
        }
    }

    #[tokio::test]
    async fn expires_more_than_10_minutes() {
        let token = Token {
            token: "token".to_string(),
            expires_at: Utc::now() + TimeDelta::minutes(35),
        };
        let token_mngr = MockTokenManager::new(token);

        let refresh_advance = 10 * 60; // 10 minutes
        let cooldown = cooldown::Backoff {
            base_secs: 30,
            growth_factor: 2,
            max_secs: 12 * 60 * 60,
        };

        // expect to wait until 10 minutes before expiration (25 minutes)
        for i in 0..10 {
            let err_streak = i;
            let actual = calc_refresh_wait(
                &token_mngr,
                &synced_clock(),
                refresh_advance,
                err_streak,
                cooldown,
            )
            .await;
            let expected = Duration::from_secs(25 * 60);
            assert!(actual < expected);
            assert!(actual > expected - Duration::from_secs(5));
        }
    }
}

pub mod wrong_system_clock {
    use super::*;

    fn backoff() -> cooldown::Backoff {
        TokenRefreshWorkerOptions::default().backoff
    }

    fn token_valid_for_a_day() -> MockTokenManager {
        MockTokenManager::new(Token {
            token: "token".to_string(),
            expires_at: Utc::now() + TimeDelta::days(1),
        })
    }

    // Before the fix, a saved token read by a clock at 1970 scheduled the next
    // refresh ~56 years out, so the token silently expired once NTP corrected
    // the clock and nothing refreshed it.
    #[tokio::test]
    async fn system_clock_at_1970_caps_the_wait_until_server_time_is_known() {
        let clock = system_clock_at(DateTime::UNIX_EPOCH);
        let wait = calc_refresh_wait(&token_valid_for_a_day(), &clock, 15 * 60, 0, backoff()).await;
        assert_eq!(wait, Duration::from_secs(backoff().max_secs as u64));
    }

    #[tokio::test]
    async fn server_time_overrides_a_system_clock_at_1970() {
        let clock = system_clock_at(DateTime::UNIX_EPOCH);
        clock.observe_server_time(Utc::now());
        let wait = calc_refresh_wait(&token_valid_for_a_day(), &clock, 15 * 60, 0, backoff()).await;
        let expected = Duration::from_secs(24 * 60 * 60 - 15 * 60);
        assert!(wait <= expected && wait > expected - Duration::from_secs(5));
    }

    #[tokio::test]
    async fn server_time_overrides_a_system_clock_far_ahead() {
        // a clock far ahead reads the token as expired and would refresh on
        // every cooldown; the server's time says it is still valid
        let clock = system_clock_at(Utc::now() + TimeDelta::days(30));
        clock.observe_server_time(Utc::now());
        let wait = calc_refresh_wait(&token_valid_for_a_day(), &clock, 15 * 60, 0, backoff()).await;
        assert!(wait > Duration::from_secs(23 * 60 * 60));
    }

    #[tokio::test]
    async fn short_waits_are_not_capped() {
        let clock = system_clock_at(Utc::now());
        let token_mngr = MockTokenManager::new(Token {
            token: "token".to_string(),
            expires_at: Utc::now() + TimeDelta::minutes(20),
        });
        let wait = calc_refresh_wait(&token_mngr, &clock, 15 * 60, 0, backoff()).await;
        assert!(wait <= Duration::from_secs(5 * 60) && wait > Duration::from_secs(4 * 60));
    }
}
