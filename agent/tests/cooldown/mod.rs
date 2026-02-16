// internal crates
use miru_agent::cooldown;

#[test]
fn test_calc_exp_backoff() {
    // growth_factor = 1 (no growth)
    let opts = cooldown::Backoff {
        base_secs: 2,
        growth_factor: 1,
        max_secs: 10,
    };
    assert_eq!(cooldown::calc(&opts, 0), 2);
    let opts = cooldown::Backoff {
        base_secs: 4,
        growth_factor: 1,
        max_secs: 10,
    };
    assert_eq!(cooldown::calc(&opts, 1), 4);
    let opts = cooldown::Backoff {
        base_secs: 11,
        growth_factor: 1,
        max_secs: 10,
    };
    assert_eq!(cooldown::calc(&opts, 2), 10); // clamped to max

    // growth_factor = 2
    let opts = cooldown::Backoff {
        base_secs: 1,
        growth_factor: 2,
        max_secs: 10,
    };
    assert_eq!(cooldown::calc(&opts, 0), 1);
    assert_eq!(cooldown::calc(&opts, 1), 2);
    assert_eq!(cooldown::calc(&opts, 3), 8);
    assert_eq!(cooldown::calc(&opts, 4), 10); // clamped to max

    // growth_factor = 4
    let opts = cooldown::Backoff {
        base_secs: 3,
        growth_factor: 4,
        max_secs: 56,
    };
    assert_eq!(cooldown::calc(&opts, 0), 3);
    assert_eq!(cooldown::calc(&opts, 1), 12);
    assert_eq!(cooldown::calc(&opts, 2), 48);
    assert_eq!(cooldown::calc(&opts, 3), 56); // clamped to max
}

#[test]
fn test_calc_exp_backoff_edge_cases() {
    // exp = 0 means growth_factor^0 = 1, so result = base * 1 = base
    let opts = cooldown::Backoff {
        base_secs: 5,
        growth_factor: 2,
        max_secs: 100,
    };
    assert_eq!(cooldown::calc(&opts, 0), 5);

    // base = 0 always returns 0
    let opts = cooldown::Backoff {
        base_secs: 0,
        growth_factor: 2,
        max_secs: 100,
    };
    assert_eq!(cooldown::calc(&opts, 5), 0);

    // overflow: large base and exponent should saturate rather than panic
    let opts = cooldown::Backoff {
        base_secs: i64::MAX,
        growth_factor: 2,
        max_secs: i64::MAX,
    };
    assert_eq!(cooldown::calc(&opts, 10), i64::MAX);
    let opts = cooldown::Backoff {
        base_secs: 1000,
        growth_factor: i64::MAX,
        max_secs: 500,
    };
    assert_eq!(cooldown::calc(&opts, 3), 500);
}
