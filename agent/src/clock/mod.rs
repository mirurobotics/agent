//! Time sources that do not depend on the robot's wall clock being right.
//!
//! Robots without a battery-backed clock boot at 1970 (or at a stale saved
//! time) and only learn the real time once NTP syncs, if it ever does. The
//! agent therefore answers three different questions about time:
//!
//! - **How long has it been?** [`Clock::instant`] and [`Clock::elapsed_since`]
//!   use the monotonic clock, which never jumps but resets on reboot. Use these
//!   for cooldowns, backoff and refresh scheduling.
//! - **What time is it, for sure?** [`Clock::trusted_now`] is the backend's
//!   time, learned from the `Date` header of the latest backend response and
//!   advanced by monotonic time since. It is `None` until the agent has heard
//!   from the backend in this process.
//! - **Roughly what time is it?** [`Clock::raw_now`] is the wall clock as-is.
//!   Use it only for logs.

pub mod http_date;
pub mod trusted;

pub use self::http_date::parse_http_date;
pub use self::trusted::{Clock, TRUST_TOLERANCE};
