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
    assert!(
        info.used_swap() <= info.tot_swap,
        "used_swap should be <= tot_swap"
    );
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
        info.free_mem() <= info.tot_mem,
        "free_mem should be <= tot_mem"
    );
    assert!(
        info.avail_mem() <= info.tot_mem,
        "avail_mem should be <= tot_mem"
    );
    assert!(
        info.used_mem() <= info.tot_mem,
        "used_mem should be <= tot_mem"
    );
    assert!(
        info.free_swap() <= info.tot_swap,
        "free_swap should be <= tot_swap"
    );
    assert!(
        info.used_swap() <= info.tot_swap,
        "used_swap should be <= tot_swap"
    );
    assert!(
        info.used_mem() > 0 || info.free_mem() > 0,
        "at least one of used_mem or free_mem should be > 0"
    );
}
