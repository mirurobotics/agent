// internal crates
use miru_agent::telemetry::SystemInfo;

#[test]
fn test_system_info_new() {
    let info = SystemInfo::new();
    assert!(!info.host_name.is_empty(), "host_name should not be empty");
    assert!(!info.arch.is_empty(), "arch should not be empty");
    assert!(!info.os.is_empty(), "os should not be empty");
    assert!(info.n_cpus > 0, "n_cpus should be > 0");
    assert!(info.tot_mem > 0, "tot_mem should be > 0");
    assert!(info.tot_swap > 0, "tot_swap should be > 0");
}

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
fn test_memory_methods() {
    let info = SystemInfo::new();
    assert!(
        info.free_mem() > 0,
        "free_mem should be < tot_mem on a running system"
    );
    assert!(
        info.avail_mem() > 0,
        "avail_mem should be < tot_mem on a running system"
    );
    assert!(
        info.used_mem() > 0,
        "used_mem should be > 0 on a running system"
    );
    assert!(
        info.free_swap() > 0,
        "free_swap should be > 0 on a running system"
    );
    assert!(
        info.used_swap() > 0,
        "used_swap should be > 0 on a running system"
    );
}
