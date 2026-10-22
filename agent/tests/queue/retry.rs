//! The shared retry policy: which failures count, how long a job waits, and
//! when it is dropped.

// internal crates
use miru_agent::cooldown::Backoff;
use miru_agent::queue::{RetryPolicy, Verdict};

// external crates
use chrono::TimeDelta;

#[derive(Debug, thiserror::Error)]
#[error("test failure")]
struct Failure {
    network: bool,
    terminal: bool,
}

impl miru_agent::errors::Error for Failure {
    fn is_network_conn_err(&self) -> bool {
        self.network
    }

    fn is_terminal(&self) -> bool {
        self.terminal
    }
}

const NETWORK: Failure = Failure {
    network: true,
    terminal: false,
};
const RETRYABLE: Failure = Failure {
    network: false,
    terminal: false,
};
const TERMINAL: Failure = Failure {
    network: false,
    terminal: true,
};

fn policy() -> RetryPolicy {
    RetryPolicy {
        attempts: 3,
        backoff: Backoff {
            base_secs: 10,
            growth_factor: 2,
            max_secs: 15,
        },
        max_age: TimeDelta::days(7),
    }
}

fn young() -> TimeDelta {
    TimeDelta::hours(1)
}

#[test]
fn defaults_match_the_data_uploads() {
    let policy = RetryPolicy::default();
    let backoff = policy.backoff;

    assert_eq!((policy.attempts, policy.max_age), (30, TimeDelta::days(7)));
    assert_eq!(
        (backoff.base_secs, backoff.growth_factor, backoff.max_secs),
        (10, 2, 3600)
    );
}

#[test]
fn a_network_error_waits_the_base_without_counting() {
    let verdict = policy().on_failure(&NETWORK, 2, young());

    assert_eq!(
        verdict,
        Verdict::Retry {
            attempts: 2,
            wait_secs: 10
        }
    );
}

#[test]
fn a_network_error_past_the_age_backstop_is_too_old() {
    let age = TimeDelta::days(7);

    assert_eq!(
        policy().on_failure(&NETWORK, 0, age),
        Verdict::TooOld { attempts: 0, age }
    );
}

#[test]
fn a_counted_failure_backs_off_with_its_attempts() {
    let first = policy().on_failure(&RETRYABLE, 0, young());
    let second = policy().on_failure(&RETRYABLE, 1, young());

    assert_eq!(
        first,
        Verdict::Retry {
            attempts: 1,
            wait_secs: 10
        }
    );
    // capped at max_secs
    assert_eq!(
        second,
        Verdict::Retry {
            attempts: 2,
            wait_secs: 15
        }
    );
}

#[test]
fn a_counted_failure_out_of_attempts_is_dropped() {
    assert_eq!(
        policy().on_failure(&RETRYABLE, 2, young()),
        Verdict::OutOfAttempts { attempts: 3 }
    );
}

#[test]
fn a_terminal_error_is_dropped_at_once() {
    assert_eq!(
        policy().on_failure(&TERMINAL, 0, young()),
        Verdict::Terminal { attempts: 1 }
    );
}

#[test]
fn a_counted_failure_past_the_age_cap_is_too_old() {
    let age = TimeDelta::days(7);

    assert_eq!(
        policy().on_failure(&RETRYABLE, 0, age),
        Verdict::TooOld { attempts: 1, age }
    );
}

#[test]
fn a_terminal_error_past_the_age_cap_is_still_terminal() {
    assert_eq!(
        policy().on_failure(&TERMINAL, 0, TimeDelta::days(30)),
        Verdict::Terminal { attempts: 1 }
    );
}

#[test]
fn a_network_error_marked_terminal_is_still_a_network_error() {
    let both = Failure {
        network: true,
        terminal: true,
    };

    assert_eq!(
        policy().on_failure(&both, 2, young()),
        Verdict::Retry {
            attempts: 2,
            wait_secs: 10
        }
    );
}
