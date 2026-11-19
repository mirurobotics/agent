// external crates
use sysinfo::System;

pub struct SystemInfo;

impl SystemInfo {
    pub fn host_name() -> String {
        System::host_name().unwrap_or_default()
    }

    pub fn os() -> String {
        System::long_os_version().unwrap_or_default()
    }

    pub fn kernel_version() -> String {
        System::kernel_version().unwrap_or_default()
    }

    pub fn arch() -> String {
        System::cpu_arch()
    }
}
