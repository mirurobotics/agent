// internal crates
use miru_daemon::telemetry::SystemInfo;

#[test]
fn test_static_methods() {
    assert!(
        !SystemInfo::host_name().is_empty(),
        "host_name should not be empty"
    );
    assert!(!SystemInfo::os().is_empty(), "os should not be empty");
    assert!(!SystemInfo::arch().is_empty(), "arch should not be empty");
}

#[test]
fn test_kernel_version_returns_stable_string() {
    // The kernel version can be blank on some hosts (e.g. restricted
    // containers), so assert only that the accessor returns a `String` and is
    // stable across calls rather than asserting it is non-empty.
    let first: String = SystemInfo::kernel_version();
    let second = SystemInfo::kernel_version();
    assert_eq!(
        first, second,
        "kernel_version should be stable across calls"
    );
}
