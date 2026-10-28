//! Time sources that do not depend on the robot's system clock being right.
//!
//! Robots without a battery-backed clock boot at 1970 (or at a stale saved
//! time) and only learn the real time once NTP syncs, if it ever does. The
//! agent therefore answers three different questions about time:
//!
//! - **How long has it been?** [`Clock::monotonic_now`] and [`Clock::elapsed_since`]
//!   use the monotonic clock, which never jumps but resets on reboot. Use these
//!   for cooldowns, backoff and refresh scheduling.
//! - **What time is it?** [`Clock::now`] gives the best available time with a
//!   [`Trust`] tier: the system clock when it agrees with the server
//!   ([`Trust::Synced`]), the server's time when it does not
//!   ([`Trust::Corrected`]), and the system clock before the server has been
//!   heard from ([`Trust::Unverified`]). [`Clock::trusted_now`] is the same
//!   time, but only for the first two tiers. Prefer these over the raw sources
//!   below.
//! - **What time does the server say it is?** [`Clock::server_now`] is the server's
//!   time, from the latest reading passed to [`Clock::observe_server_time`]
//!   and advanced by monotonic time since. It is `None` until the agent has heard
//!   from the server in this process.
//!   In the agent the server is the Miru backend: `http::Client` reads its
//!   time from the `Date` header of backend responses and records it here.
//! - **Roughly what time is it?** [`Clock::system_now`] is the system clock as-is.
//!   Use it only for logs.

pub mod shared;

pub use self::shared::{Clock, Reading, Trust, SYNC_TOLERANCE};
