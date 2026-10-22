// internal crates
use miru_agent::http::date;

// external crates
use chrono::{DateTime, TimeDelta, TimeZone, Utc};

const DATE: &str = "Tue, 20 Oct 2026 02:58:04 GMT";

fn date_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 20, 2, 58, 4).unwrap()
}

pub mod server_time {
    use super::*;

    #[test]
    fn is_the_date_when_there_is_no_age() {
        assert_eq!(date::server_time(DATE, None), Some(date_time()));
    }

    #[test]
    fn adds_a_cached_responses_age() {
        assert_eq!(
            date::server_time(DATE, Some("300")),
            Some(date_time() + TimeDelta::seconds(300))
        );
    }

    #[test]
    fn treats_a_bad_age_as_zero() {
        assert_eq!(date::server_time(DATE, Some("soon")), Some(date_time()));
    }

    #[test]
    fn clamps_an_age_past_the_end_of_time() {
        let max_secs = TimeDelta::MAX.num_seconds().to_string();
        assert_eq!(
            date::server_time(DATE, Some(&max_secs)),
            Some(DateTime::<Utc>::MAX_UTC)
        );
    }

    #[test]
    fn is_none_for_an_invalid_date() {
        assert_eq!(date::server_time("yesterday", Some("10")), None);
    }
}

pub mod parse_http_date {
    use super::*;

    #[test]
    fn parses_imf_fixdate() {
        assert_eq!(date::parse_http_date(DATE), Some(date_time()));
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(date::parse_http_date("not a date"), None);
        assert_eq!(date::parse_http_date(""), None);
    }
}

pub mod parse_age {
    use super::*;

    #[test]
    fn parses_whole_seconds() {
        assert_eq!(date::parse_age("0"), Some(TimeDelta::zero()));
        assert_eq!(date::parse_age(" 120 "), Some(TimeDelta::seconds(120)));
    }

    #[test]
    fn rejects_bad_ages() {
        assert_eq!(date::parse_age("-5"), None);
        assert_eq!(date::parse_age("1.5"), None);
        assert_eq!(date::parse_age("soon"), None);
        assert_eq!(date::parse_age(&i64::MAX.to_string()), None);
    }
}
