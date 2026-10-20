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

pub mod server_now {
    use super::*;

    #[test]
    fn is_none_before_any_server_reading() {
        let fake = Fake::new(real());
        assert_eq!(fake.clock.server_now(), None);
        assert_eq!(fake.clock.offset(), None);
        assert!(!fake.clock.is_system_clock_synced());
    }

    #[test]
    fn is_the_server_time_when_the_system_clock_is_at_1970() {
        let fake = Fake::new(epoch());
        fake.clock.observe_server_time(real());
        assert_eq!(fake.clock.server_now(), Some(real()));
        assert_eq!(fake.clock.offset(), Some(real() - epoch()));
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

pub mod system_clock_sync {
    use super::*;

    #[test]
    fn synced_when_within_tolerance() {
        let fake = Fake::new(real());
        fake.clock
            .observe_server_time(real() + clock::SYNC_TOLERANCE);
        assert!(fake.clock.is_system_clock_synced());
    }

    #[test]
    fn not_synced_when_past_tolerance() {
        let fake = Fake::new(real());
        fake.clock
            .observe_server_time(real() + clock::SYNC_TOLERANCE + TimeDelta::seconds(1));
        assert!(!fake.clock.is_system_clock_synced());
    }

    #[test]
    fn becomes_synced_once_ntp_corrects_the_system_clock() {
        let fake = Fake::new(epoch());
        fake.clock.observe_server_time(real());
        assert!(!fake.clock.is_system_clock_synced());
        fake.advance(Duration::from_secs(30));
        fake.jump_system_clock(real() + TimeDelta::seconds(30));
        assert!(fake.clock.is_system_clock_synced());
    }
}

pub mod elapsed {
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

    #[test]
    fn system_now_is_the_system_clock() {
        let fake = Fake::new(epoch());
        assert_eq!(fake.clock.system_now(), epoch());
    }

    #[test]
    fn system_clock_reads_the_system_time() {
        let clock = Clock::default();
        let before = Utc::now();
        let system_now = clock.system_now();
        assert!(system_now >= before && system_now <= Utc::now());
        let start = clock.monotonic_now();
        assert!(clock.elapsed_since(start) < Duration::from_secs(5));
        assert!(format!("{clock:?}").contains("Clock"));
    }
}
