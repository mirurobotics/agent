// standard crates
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// internal crates
use miru_agent::clock::{self, Clock};

// external crates
use chrono::{DateTime, TimeDelta, TimeZone, Utc};

/// A clock whose wall clock and monotonic clock tests move by hand.
struct Fake {
    raw: Arc<Mutex<DateTime<Utc>>>,
    mono: Arc<Mutex<Instant>>,
    clock: Clock,
}

impl Fake {
    fn new(raw: DateTime<Utc>) -> Self {
        let raw = Arc::new(Mutex::new(raw));
        let mono = Arc::new(Mutex::new(Instant::now()));
        let (r, m) = (raw.clone(), mono.clone());
        let clock = Clock::from_sources(
            Arc::new(move || *r.lock().unwrap()),
            Arc::new(move || *m.lock().unwrap()),
        );
        Self { raw, mono, clock }
    }

    /// Advance both clocks together, as real time passing does.
    fn advance(&self, d: Duration) {
        *self.mono.lock().unwrap() += d;
        *self.raw.lock().unwrap() += TimeDelta::from_std(d).unwrap();
    }

    /// Jump only the wall clock, as NTP correcting it does.
    fn jump_wall(&self, to: DateTime<Utc>) {
        *self.raw.lock().unwrap() = to;
    }
}

fn epoch() -> DateTime<Utc> {
    DateTime::UNIX_EPOCH
}

fn real() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 20, 2, 58, 4).unwrap()
}

pub mod trusted_now {
    use super::*;

    #[test]
    fn is_none_before_any_backend_reading() {
        let fake = Fake::new(real());
        assert_eq!(fake.clock.trusted_now(), None);
        assert_eq!(fake.clock.offset(), None);
        assert!(!fake.clock.is_wall_clock_trusted());
    }

    #[test]
    fn is_the_backend_time_when_the_wall_clock_is_at_1970() {
        let fake = Fake::new(epoch());
        fake.clock.observe_server_time(real());
        assert_eq!(fake.clock.trusted_now(), Some(real()));
        assert_eq!(fake.clock.offset(), Some(real() - epoch()));
    }

    #[test]
    fn advances_with_monotonic_time() {
        let fake = Fake::new(epoch());
        fake.clock.observe_server_time(real());
        fake.advance(Duration::from_secs(90));
        assert_eq!(
            fake.clock.trusted_now(),
            Some(real() + TimeDelta::seconds(90))
        );
    }

    #[test]
    fn ignores_wall_clock_jumps() {
        let fake = Fake::new(epoch());
        fake.clock.observe_server_time(real());
        fake.jump_wall(real() + TimeDelta::days(400));
        assert_eq!(fake.clock.trusted_now(), Some(real()));
    }

    #[test]
    fn latest_reading_wins() {
        let fake = Fake::new(real());
        fake.clock.observe_server_time(real());
        let later = real() + TimeDelta::hours(1);
        fake.clock.observe_server_time(later);
        assert_eq!(fake.clock.trusted_now(), Some(later));
    }

    #[test]
    fn clones_share_the_reading() {
        let fake = Fake::new(real());
        let other = fake.clock.clone();
        other.observe_server_time(real());
        assert_eq!(fake.clock.trusted_now(), Some(real()));
    }
}

pub mod wall_clock_trust {
    use super::*;

    #[test]
    fn trusted_when_within_tolerance() {
        let fake = Fake::new(real());
        fake.clock
            .observe_server_time(real() + clock::TRUST_TOLERANCE);
        assert!(fake.clock.is_wall_clock_trusted());
    }

    #[test]
    fn untrusted_when_past_tolerance() {
        let fake = Fake::new(real());
        fake.clock
            .observe_server_time(real() + clock::TRUST_TOLERANCE + TimeDelta::seconds(1));
        assert!(!fake.clock.is_wall_clock_trusted());
    }

    #[test]
    fn becomes_trusted_once_ntp_corrects_the_wall_clock() {
        let fake = Fake::new(epoch());
        fake.clock.observe_server_time(real());
        assert!(!fake.clock.is_wall_clock_trusted());
        fake.advance(Duration::from_secs(30));
        fake.jump_wall(real() + TimeDelta::seconds(30));
        assert!(fake.clock.is_wall_clock_trusted());
    }
}

pub mod elapsed {
    use super::*;

    #[test]
    fn measures_monotonic_time_not_wall_time() {
        let fake = Fake::new(epoch());
        let start = fake.clock.instant();
        fake.jump_wall(real());
        assert_eq!(fake.clock.elapsed_since(start), Duration::ZERO);
        fake.advance(Duration::from_secs(5));
        assert_eq!(fake.clock.elapsed_since(start), Duration::from_secs(5));
    }

    #[test]
    fn is_zero_for_a_future_instant() {
        let fake = Fake::new(real());
        let future = fake.clock.instant() + Duration::from_secs(10);
        assert_eq!(fake.clock.elapsed_since(future), Duration::ZERO);
    }

    #[test]
    fn raw_now_is_the_wall_clock() {
        let fake = Fake::new(epoch());
        assert_eq!(fake.clock.raw_now(), epoch());
    }

    #[test]
    fn system_clock_reads_the_system_time() {
        let clock = Clock::default();
        let before = Utc::now();
        let raw = clock.raw_now();
        assert!(raw >= before && raw <= Utc::now());
        let start = clock.instant();
        assert!(clock.elapsed_since(start) < Duration::from_secs(5));
        assert!(format!("{clock:?}").contains("Clock"));
    }
}

pub mod date_header {
    use super::*;

    #[test]
    fn parses_imf_fixdate() {
        assert_eq!(
            clock::parse_http_date("Tue, 20 Oct 2026 02:58:04 GMT"),
            Some(real())
        );
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(clock::parse_http_date("not a date"), None);
        assert_eq!(clock::parse_http_date(""), None);
    }

    #[test]
    fn parses_age_seconds() {
        assert_eq!(clock::parse_age("0"), Some(TimeDelta::zero()));
        assert_eq!(clock::parse_age(" 120 "), Some(TimeDelta::seconds(120)));
    }

    #[test]
    fn rejects_bad_ages() {
        assert_eq!(clock::parse_age("-5"), None);
        assert_eq!(clock::parse_age("1.5"), None);
        assert_eq!(clock::parse_age("soon"), None);
        assert_eq!(clock::parse_age(&i64::MAX.to_string()), None);
    }

    #[test]
    fn observe_records_a_valid_header() {
        let fake = Fake::new(epoch());
        fake.clock
            .observe_date_header("Tue, 20 Oct 2026 02:58:04 GMT", None);
        assert_eq!(fake.clock.trusted_now(), Some(real()));
    }

    #[test]
    fn observe_adds_a_cached_responses_age() {
        let fake = Fake::new(epoch());
        fake.clock
            .observe_date_header("Tue, 20 Oct 2026 02:58:04 GMT", Some("300"));
        assert_eq!(
            fake.clock.trusted_now(),
            Some(real() + TimeDelta::seconds(300))
        );
    }

    #[test]
    fn observe_treats_a_bad_age_as_zero() {
        let fake = Fake::new(epoch());
        fake.clock
            .observe_date_header("Tue, 20 Oct 2026 02:58:04 GMT", Some("soon"));
        assert_eq!(fake.clock.trusted_now(), Some(real()));
    }

    #[test]
    fn observe_clamps_an_age_past_the_end_of_time() {
        let fake = Fake::new(epoch());
        let max_secs = (TimeDelta::MAX.num_seconds()).to_string();
        fake.clock
            .observe_date_header("Tue, 20 Oct 2026 02:58:04 GMT", Some(&max_secs));
        assert_eq!(fake.clock.trusted_now(), Some(DateTime::<Utc>::MAX_UTC));
    }

    #[test]
    fn observe_ignores_an_invalid_header() {
        let fake = Fake::new(epoch());
        fake.clock.observe_date_header("yesterday", Some("10"));
        assert_eq!(fake.clock.trusted_now(), None);
    }
}
