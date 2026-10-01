//! Per-OS filesystem path defaults.
//!
//! The OS-specific functions are compiled on every target so their logic is
//! unit-testable from any host; only the dispatchers ([`data_root_base`],
//! [`log_dir`]) read the real environment.

// standard crates
use std::ffi::OsString;
use std::path::PathBuf;

/// Base directory the agent's data [`Layout`](crate::disk::Layout) roots
/// under: `/` on Unix (the layout appends `var/lib/miru`), `%ProgramData%`
/// on Windows (the layout appends `Miru`).
pub fn data_root_base() -> PathBuf {
    #[cfg(unix)]
    {
        unix_data_root_base()
    }
    #[cfg(windows)]
    {
        windows_data_root_base(std::env::var_os("ProgramData"))
    }
}

pub fn unix_data_root_base() -> PathBuf {
    PathBuf::from("/")
}

/// Path segments appended to [`data_root_base`] to form the agent's data root:
/// `var/lib/miru` on Unix, `Miru` on Windows (under `%ProgramData%`).
pub fn data_root_suffix() -> PathBuf {
    #[cfg(unix)]
    {
        PathBuf::from("var").join("lib").join("miru")
    }
    #[cfg(windows)]
    {
        PathBuf::from("Miru")
    }
}

/// `program_data` is the value of the `ProgramData` environment variable;
/// `C:\ProgramData` when unset.
pub fn windows_data_root_base(program_data: Option<OsString>) -> PathBuf {
    PathBuf::from(program_data.unwrap_or_else(|| OsString::from(r"C:\ProgramData")))
}

/// Default directory for agent log files.
pub fn log_dir() -> PathBuf {
    #[cfg(unix)]
    {
        unix_log_dir()
    }
    #[cfg(windows)]
    {
        windows_log_dir(std::env::var_os("ProgramData"))
    }
}

pub fn unix_log_dir() -> PathBuf {
    PathBuf::from("/var/log/miru")
}

pub fn windows_log_dir(program_data: Option<OsString>) -> PathBuf {
    windows_data_root_base(program_data)
        .join("Miru")
        .join("logs")
}
