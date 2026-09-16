// standard crates
use std::ffi::OsString;
use std::path::PathBuf;

// internal crates
use miru_agent::platform;

pub mod unix_defaults {
    use super::*;

    #[test]
    fn data_root_base_is_filesystem_root() {
        assert_eq!(platform::unix_data_root_base(), PathBuf::from("/"));
    }

    #[test]
    fn log_dir_is_var_log_miru() {
        assert_eq!(platform::unix_log_dir(), PathBuf::from("/var/log/miru"));
    }
}

pub mod windows_defaults {
    use super::*;

    #[test]
    fn data_root_base_falls_back_to_program_data_default() {
        assert_eq!(
            platform::windows_data_root_base(None),
            PathBuf::from(r"C:\ProgramData"),
        );
    }

    #[test]
    fn data_root_base_honors_program_data_env_value() {
        assert_eq!(
            platform::windows_data_root_base(Some(OsString::from(r"D:\CustomData"))),
            PathBuf::from(r"D:\CustomData"),
        );
    }

    #[test]
    fn log_dir_nests_miru_logs_under_program_data() {
        let expected: PathBuf = [
            OsString::from(r"D:\CustomData"),
            "Miru".into(),
            "logs".into(),
        ]
        .iter()
        .collect();
        assert_eq!(
            platform::windows_log_dir(Some(OsString::from(r"D:\CustomData"))),
            expected,
        );
    }

    #[test]
    fn log_dir_falls_back_to_program_data_default() {
        let expected: PathBuf = [
            OsString::from(r"C:\ProgramData"),
            "Miru".into(),
            "logs".into(),
        ]
        .iter()
        .collect();
        assert_eq!(platform::windows_log_dir(None), expected);
    }
}

pub mod dispatch {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn data_root_base_matches_unix_default() {
        assert_eq!(platform::data_root_base(), platform::unix_data_root_base());
    }

    #[cfg(unix)]
    #[test]
    fn log_dir_matches_unix_default() {
        assert_eq!(platform::log_dir(), platform::unix_log_dir());
    }

    #[cfg(windows)]
    #[test]
    fn data_root_base_reads_program_data() {
        assert_eq!(
            platform::data_root_base(),
            platform::windows_data_root_base(std::env::var_os("ProgramData")),
        );
    }

    #[cfg(windows)]
    #[test]
    fn log_dir_reads_program_data() {
        assert_eq!(
            platform::log_dir(),
            platform::windows_log_dir(std::env::var_os("ProgramData")),
        );
    }

    #[cfg(unix)]
    #[test]
    fn supports_idle_exit_on_unix() {
        assert!(platform::supports_idle_exit());
    }

    #[cfg(windows)]
    #[test]
    fn does_not_support_idle_exit_on_windows() {
        assert!(!platform::supports_idle_exit());
    }
}
