// standard crates
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// internal crates
use miru_agent::clock::{self, Clock};

// external crates
use chrono::{DateTime, TimeDelta, TimeZone, Utc};

/// A clock whose system clock and monotonic clock tests move by hand.
struct Fake {
    system: Arc<Mutex<DateTime<Utc>>>,
    monotonic: Arc<Mutex<Instant>>,
    clock: Clock,
}

impl Fake {
    fn new(system: DateTime<Utc>) -> Self {
        let system = Arc::new(Mutex::new(system));
        let monotonic = Arc::new(Mutex::new(Instant::now()));
        let (r, m) = (system.clone(), monotonic.clone());
        let clock = Clock::from_sources(
            Arc::new(move || *r.lock().unwrap()),
            Arc::new(move || *m.lock().unwrap()),
        );
        Self {
            system,
            monotonic,
            clock,
        }
    }

    /// Advance both clocks together, as real time passing does.
    fn advance(&self, d: Duration) {
        *self.monotonic.lock().unwrap() += d;
        *self.system.lock().unwrap() += TimeDelta::from_std(d).unwrap();
    }

    /// Jump only the system clock, as NTP correcting it does.
    fn jump_system_clock(&self, to: DateTime<Utc>) {
        *self.system.lock().unwrap() = to;
    }
}

fn epoch() -> DateTime<Utc> {
    DateTime::UNIX_EPOCH
}

fn real() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 20, 2, 58, 4).unwrap()
}

pub mod new {
    use super::*;

    #[test]
    fn reads_the_real_system_and_monotonic_clocks() {
        let clock = Clock::default();
        let before = Utc::now();
        let system_now = clock.system_now();
        assert!(system_now >= before && system_now <= Utc::now());
        let start = clock.monotonic_now();
        assert!(clock.elapsed_since(start) < Duration::from_secs(5));
        assert!(format!("{clock:?}").contains("Clock"));
    }
}

pub mod system_now {
    use super::*;

    #[test]
    fn is_the_system_clock() {
        let fake = Fake::new(epoch());
        assert_eq!(fake.clock.system_now(), epoch());
    }
}

pub mod monotonic_now {
    use super::*;

    #[test]
    fn is_the_monotonic_clock() {
        let fake = Fake::new(epoch());
        let start = fake.clock.monotonic_now();
        fake.advance(Duration::from_secs(7));
        assert_eq!(fake.clock.monotonic_now(), start + Duration::from_secs(7));
    }

    #[test]
    fn ignores_system_clock_jumps() {
        let fake = Fake::new(epoch());
        let start = fake.clock.monotonic_now();
        fake.jump_system_clock(real());
        assert_eq!(fake.clock.monotonic_now(), start);
    }
}

pub mod elapsed_since {
    use super::*;

    #[test]
    fn measures_monotonic_time_not_system_time() {
        let fake = Fake::new(epoch());
        let start = fake.clock.monotonic_now();
        fake.jump_system_clock(real());
        assert_eq!(fake.clock.elapsed_since(start), Duration::ZERO);
        fake.advance(Duration::from_secs(5));
        assert_eq!(fake.clock.elapsed_since(start), Duration::from_secs(5));
    }

    #[test]
    fn is_zero_for_a_future_instant() {
        let fake = Fake::new(real());
        let future = fake.clock.monotonic_now() + Duration::from_secs(10);
        assert_eq!(fake.clock.elapsed_since(future), Duration::ZERO);
    }
}

pub mod observe_server_time {
    use super::*;

    #[test]
    fn latest_reading_wins() {
        let fake = Fake::new(real());
        fake.clock.observe_server_time(real());
        let later = real() + TimeDelta::hours(1);
        fake.clock.observe_server_time(later);
        assert_eq!(fake.clock.server_now(), Some(later));
    }

    #[test]
    fn clones_share_the_reading() {
        let fake = Fake::new(real());
        let other = fake.clock.clone();
        other.observe_server_time(real());
        assert_eq!(fake.clock.server_now(), Some(real()));
    }
}

pub mod server_now {
    use super::*;

    #[test]
    fn is_none_before_any_server_reading() {
        let fake = Fake::new(real());
        assert_eq!(fake.clock.server_now(), None);
    }

    #[test]
    fn is_the_server_time_when_the_system_clock_is_at_1970() {
        let fake = Fake::new(epoch());
        fake.clock.observe_server_time(real());
        assert_eq!(fake.clock.server_now(), Some(real()));
    }

    #[test]
    fn advances_with_monotonic_time() {
        let fake = Fake::new(epoch());
        fake.clock.observe_server_time(real());
        fake.advance(Duration::from_secs(90));
        assert_eq!(
            fake.clock.server_now(),
            Some(real() + TimeDelta::seconds(90))
        );
    }

    #[test]
    fn ignores_system_clock_jumps() {
        let fake = Fake::new(epoch());
        fake.clock.observe_server_time(real());
        fake.jump_system_clock(real() + TimeDelta::days(400));
        assert_eq!(fake.clock.server_now(), Some(real()));
    }
}

pub mod now {
    use super::*;
    use miru_agent::clock::{Reading, Trust};

    #[test]
    fn is_the_unverified_system_clock_before_any_server_reading() {
        let fake = Fake::new(epoch());
        let reading = fake.clock.now();
        assert_eq!(
            reading,
            Reading {
                time: epoch(),
                trust: Trust::Unverified
            }
        );
        assert!(!reading.is_trusted());
    }

    #[test]
    fn is_the_synced_system_clock_when_it_agrees_with_the_server() {
        let fake = Fake::new(real());
        fake.clock
            .observe_server_time(real() + clock::SYNC_TOLERANCE);
        let reading = fake.clock.now();
        assert_eq!(
            reading,
            Reading {
                time: real(),
                trust: Trust::Synced
            }
        );
        assert!(reading.is_trusted());
    }

    #[test]
    fn is_the_corrected_server_time_when_the_system_clock_disagrees() {
        let fake = Fake::new(epoch());
        fake.clock.observe_server_time(real());
        let reading = fake.clock.now();
        assert_eq!(
            reading,
            Reading {
                time: real(),
                trust: Trust::Corrected
            }
        );
        assert!(reading.is_trusted());
    }

    #[test]
    fn becomes_synced_once_ntp_corrects_the_system_clock() {
        let fake = Fake::new(epoch());
        fake.clock.observe_server_time(real());
        fake.advance(Duration::from_secs(30));
        fake.jump_system_clock(real() + TimeDelta::seconds(31));
        assert_eq!(
            fake.clock.now(),
            Reading {
                time: real() + TimeDelta::seconds(31),
                trust: Trust::Synced
            }
        );
    }
}

pub mod trusted_now {
    use super::*;

    #[test]
    fn is_none_before_any_server_reading() {
        let fake = Fake::new(real());
        assert_eq!(fake.clock.trusted_now(), None);
    }

    #[test]
    fn is_the_system_clock_when_synced() {
        let fake = Fake::new(real());
        fake.clock
            .observe_server_time(real() + TimeDelta::seconds(5));
        assert_eq!(fake.clock.trusted_now(), Some(real()));
    }

    #[test]
    fn is_the_server_time_when_the_system_clock_disagrees() {
        let fake = Fake::new(epoch());
        fake.clock.observe_server_time(real());
        assert_eq!(fake.clock.trusted_now(), Some(real()));
    }
}

pub mod offset {
    use super::*;

    #[test]
    fn is_none_before_any_server_reading() {
        let fake = Fake::new(real());
        assert_eq!(fake.clock.offset(), None);
    }

    #[test]
    fn is_server_time_minus_system_time() {
        let fake = Fake::new(epoch());
        fake.clock.observe_server_time(real());
        assert_eq!(fake.clock.offset(), Some(real() - epoch()));
    }
}

pub mod is_system_clock_synced {
    use super::*;

    #[test]
    fn is_false_before_any_server_reading() {
        let fake = Fake::new(real());
        assert!(!fake.clock.is_system_clock_synced());
    }

    #[test]
    fn is_true_within_tolerance() {
        let fake = Fake::new(real());
        fake.clock
            .observe_server_time(real() + clock::SYNC_TOLERANCE);
        assert!(fake.clock.is_system_clock_synced());
    }

    #[test]
    fn is_false_past_tolerance() {
        let fake = Fake::new(real());
        fake.clock
            .observe_server_time(real() + clock::SYNC_TOLERANCE + TimeDelta::seconds(1));
        assert!(!fake.clock.is_system_clock_synced());
    }

    #[test]
    fn becomes_true_once_ntp_corrects_the_system_clock() {
        let fake = Fake::new(epoch());
        fake.clock.observe_server_time(real());
        assert!(!fake.clock.is_system_clock_synced());
        fake.advance(Duration::from_secs(30));
        fake.jump_system_clock(real() + TimeDelta::seconds(30));
        assert!(fake.clock.is_system_clock_synced());
    }
}
