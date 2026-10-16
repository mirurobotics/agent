// standard crates
use std::time::Duration;

/// `{sent} of {total} ({pct}%) sent`, or just `{sent} sent` when `total` is 0.
pub(super) fn position(total: u64, sent: u64) -> String {
    if total == 0 {
        return format!("{} sent", mib(sent));
    }
    let pct = sent as f64 * 100.0 / total as f64;
    format!("{} of {} ({pct:.1}%) sent", mib(sent), mib(total))
}

/// Bytes per second over `over`, or 0 for a zero-length span.
pub(super) fn rate(bytes: u64, over: Duration) -> u64 {
    let millis = over.as_millis();
    if millis == 0 {
        return 0;
    }
    (u128::from(bytes) * 1000 / millis) as u64
}

pub(super) fn mib(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
}

pub(super) fn duration(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m{:02}s", secs / 60, secs % 60),
        _ => format!("{}h{:02}m", secs / 3600, secs % 3600 / 60),
    }
}
