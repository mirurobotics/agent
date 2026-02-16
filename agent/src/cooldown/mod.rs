// standard library
use std::cmp::min;

#[derive(Debug, Clone, Copy)]
pub struct Backoff {
    pub base_secs: i64,
    pub growth_factor: i64,
    pub max_secs: i64,
}

pub fn calc(backoff: &Backoff, exp: u32) -> i64 {
    let calculated = backoff
        .base_secs
        .saturating_mul(backoff.growth_factor.saturating_pow(exp));
    min(calculated, backoff.max_secs)
}
