// standard library
use std::sync::Arc;
use std::time::Duration;

// internal crates
use miru_agent::authn::{errors::*, token::Token};
use miru_agent::trace;
use miru_agent::utils::calc_exp_backoff;
use miru_agent::utils::CooldownOptions;
use miru_agent::workers::token_refresh::{
    calc_refresh_wait, run_token_refresh_worker, TokenRefreshWorkerOptions,
};

use crate::authn::mock::MockTokenManager;
use crate::mock::SleepController;

// external crates
use chrono::{TimeDelta, Utc};

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
        let cooldown = CooldownOptions {
            base_secs: 30,
            ..Default::default()
        };

        // run the worker
        let token_mngr_for_spawn = token_mngr.clone();
        let sleep_ctrl_for_spawn = sleep_ctrl.clone();
        let options = TokenRefreshWorkerOptions {
            refresh_advance_secs,
            polling: cooldown,
        };
        let token_refresh_handle = tokio::spawn(async move {
            run_token_refresh_worker(
                &options,
                token_mngr_for_spawn.as_ref(),
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
                is_network_connection_error: true,
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
        let cooldown = CooldownOptions {
            base_secs: 30,
            ..Default::default()
        };

        // run the worker
        let token_mngr_for_spawn = token_mngr.clone();
        let sleep_ctrl_for_spawn = sleep_ctrl.clone();
        let options = TokenRefreshWorkerOptions {
            refresh_advance_secs,
            polling: cooldown,
        };
        let token_refresh_handle = tokio::spawn(async move {
            run_token_refresh_worker(
                &options,
                token_mngr_for_spawn.as_ref(),
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
                is_network_connection_error: false,
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
        let cooldown = CooldownOptions {
            base_secs: 30,
            ..Default::default()
        };

        // run the worker
        let token_mngr_for_spawn = token_mngr.clone();
        let sleep_ctrl_for_spawn = sleep_ctrl.clone();
        let options = TokenRefreshWorkerOptions {
            refresh_advance_secs,
            polling: cooldown,
        };
        let token_refresh_handle = tokio::spawn(async move {
            run_token_refresh_worker(
                &options,
                token_mngr_for_spawn.as_ref(),
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
            let expected_sleep_secs = calc_exp_backoff(
                cooldown.base_secs,
                cooldown.growth_factor,
                i + 1,
                cooldown.max_secs,
            );
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
                is_network_connection_error: false,
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
        let cooldown = CooldownOptions {
            base_secs: 30,
            ..Default::default()
        };

        // run the worker
        let token_mngr_for_spawn = token_mngr.clone();
        let sleep_ctrl_for_spawn = sleep_ctrl.clone();
        let options = TokenRefreshWorkerOptions {
            refresh_advance_secs,
            polling: cooldown,
        };
        let token_refresh_handle = tokio::spawn(async move {
            run_token_refresh_worker(
                &options,
                token_mngr_for_spawn.as_ref(),
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
            let expected_sleep_secs = calc_exp_backoff(
                cooldown.base_secs,
                cooldown.growth_factor,
                i + 1,
                cooldown.max_secs,
            );
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
        let cooldown = CooldownOptions {
            base_secs: 30,
            ..Default::default()
        };

        for i in 0..10 {
            let err_streak = i;
            let actual =
                calc_refresh_wait(&token_mngr, refresh_advance, err_streak, cooldown).await;
            let expected_secs = calc_exp_backoff(
                cooldown.base_secs,
                cooldown.growth_factor,
                err_streak,
                cooldown.max_secs,
            );
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
        let cooldown = CooldownOptions {
            base_secs: 30,
            ..Default::default()
        };

        for i in 0..10 {
            let err_streak = i;
            let sleep_duration =
                calc_refresh_wait(&token_mngr, refresh_advance, err_streak, cooldown).await;
            let expected_secs = calc_exp_backoff(
                cooldown.base_secs,
                cooldown.growth_factor,
                err_streak,
                cooldown.max_secs,
            );
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
        let cooldown = CooldownOptions {
            base_secs: 30,
            ..Default::default()
        };

        // expect to wait until 10 minutes before expiration (25 minutes)
        for i in 0..10 {
            let err_streak = i;
            let actual =
                calc_refresh_wait(&token_mngr, refresh_advance, err_streak, cooldown).await;
            let expected = Duration::from_secs(25 * 60);
            assert!(actual < expected);
            assert!(actual > expected - Duration::from_secs(5));
        }
    }
}
