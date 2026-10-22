// standard crates
use std::time::Duration;

// internal crates
use crate::test_utils::clock::FakeClock;
use miru_agent::deploy::Cooldowns;

// external crates
use chrono::{TimeDelta, Utc};

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

#[test]
fn no_cooldown_by_default() {
    let cooldowns = Cooldowns::default();
    assert_eq!(cooldowns.remaining("dpl_1"), None);
}

#[test]
fn remaining_counts_down_on_the_monotonic_clock() {
    let clock = FakeClock::new(Utc::now());
    let cooldowns = Cooldowns::new(clock.clock());

    cooldowns.start("dpl_1", secs(60));
    assert_eq!(cooldowns.remaining("dpl_1"), Some(secs(60)));

    clock.advance_secs(59);
    assert_eq!(cooldowns.remaining("dpl_1"), Some(secs(1)));

    // the deadline itself ends the cooldown
    clock.advance_secs(1);
    assert_eq!(cooldowns.remaining("dpl_1"), None);
}

#[test]
fn a_system_clock_step_does_not_move_the_deadline() {
    let clock = FakeClock::new(Utc::now());
    let cooldowns = Cooldowns::new(clock.clock());
    cooldowns.start("dpl_1", secs(60));

    clock.set_system(Utc::now() + TimeDelta::days(365));
    assert_eq!(cooldowns.remaining("dpl_1"), Some(secs(60)));

    clock.set_system(Utc::now() - TimeDelta::days(365 * 50));
    assert_eq!(cooldowns.remaining("dpl_1"), Some(secs(60)));
}

#[test]
fn start_replaces_and_zero_clears() {
    let clock = FakeClock::new(Utc::now());
    let cooldowns = Cooldowns::new(clock.clock());

    cooldowns.start("dpl_1", secs(60));
    cooldowns.start("dpl_1", secs(10));
    assert_eq!(cooldowns.remaining("dpl_1"), Some(secs(10)));

    cooldowns.start("dpl_1", Duration::ZERO);
    assert_eq!(cooldowns.remaining("dpl_1"), None);
}

#[test]
fn clear_ends_only_that_deployment() {
    let clock = FakeClock::new(Utc::now());
    let cooldowns = Cooldowns::new(clock.clock());
    cooldowns.start("dpl_1", secs(60));
    cooldowns.start("dpl_2", secs(60));

    cooldowns.clear("dpl_1");
    cooldowns.clear("dpl_unknown");

    assert_eq!(cooldowns.remaining("dpl_1"), None);
    assert_eq!(cooldowns.remaining("dpl_2"), Some(secs(60)));
}

#[test]
fn clones_share_cooldowns() {
    let clock = FakeClock::new(Utc::now());
    let cooldowns = Cooldowns::new(clock.clock());
    let clone = cooldowns.clone();

    clone.start("dpl_1", secs(60));
    assert_eq!(cooldowns.remaining("dpl_1"), Some(secs(60)));
}
