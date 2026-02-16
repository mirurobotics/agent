use sysinfo::System;
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};

#[derive(Debug)]
pub struct SystemInfo {
    system: System,
    pub host_name: String,
    pub arch: String,
    pub os: String,
    pub n_cpus: usize,
    pub tot_mem: u64,
    pub tot_swap: u64,
}

impl SystemInfo {
    pub fn new() -> Self {
        // create a new system
        let sys = System::new_all();

        // gather system info
        SystemInfo {
            host_name: Self::host_name(),
            arch: Self::arch(),
            os: Self::os(),
            n_cpus: sys.cpus().len(),
            tot_mem: sys.total_memory(),
            tot_swap: sys.total_swap(),
            system: sys,
        }
    }

    pub fn host_name() -> String {
        System::host_name().unwrap_or_default()
    }

    pub fn os() -> String {
        System::long_os_version().unwrap_or_default()
    }

    pub fn arch() -> String {
        System::cpu_arch()
    }

    pub fn free_mem(&self) -> u64 {
        self.system.free_memory()
    }

    pub fn avail_mem(&self) -> u64 {
        self.system.available_memory()
    }

    pub fn used_mem(&self) -> u64 {
        self.system.used_memory()
    }

    pub fn free_swap(&self) -> u64 {
        self.system.free_swap()
    }

    pub fn used_swap(&self) -> u64 {
        self.system.used_swap()
    }
}
