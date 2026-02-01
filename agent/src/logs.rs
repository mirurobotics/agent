// standard library
use std::fmt::Display;
use std::path::PathBuf;

// external crates
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, EnvFilter};

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn variants() -> Vec<LogLevel> {
        vec![
            LogLevel::Trace,
            LogLevel::Debug,
            LogLevel::Info,
            LogLevel::Warn,
            LogLevel::Error,
        ]
    }
}

impl<'de> Deserialize<'de> for LogLevel {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let default = LogLevel::default();

        let result = String::deserialize(deserializer);
        let s = match result {
            Ok(s) => s,
            Err(e) => {
                error!("Error deserializing log level: {:?}", e);
                return Ok(default);
            }
        };
        match s.to_lowercase().as_str() {
            "trace" => Ok(LogLevel::Trace),
            "debug" => Ok(LogLevel::Debug),
            "info" => Ok(LogLevel::Info),
            "warn" | "warning" => Ok(LogLevel::Warn),
            "error" => Ok(LogLevel::Error),
            _ => {
                error!(
                    "Invalid log level: {}. Setting to default: '{}'",
                    s, default
                );
                Ok(default)
            }
        }
    }
}

impl Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogLevel::Trace => write!(f, "trace"),
            LogLevel::Debug => write!(f, "debug"),
            LogLevel::Info => write!(f, "info"),
            LogLevel::Warn => write!(f, "warn"),
            LogLevel::Error => write!(f, "error"),
        }
    }
}

pub struct LogOptions {
    pub stdout: bool,
    pub log_level: LogLevel,
    pub log_dir: PathBuf,
}

impl Default for LogOptions {
    fn default() -> Self {
        Self {
            stdout: true,
            log_level: LogLevel::Info,
            log_dir: PathBuf::from("/var/log/miru"),
        }
    }
}

/// Initialize the application. This function creates a logger and initialized the
/// Miru application context.
pub fn init(options: LogOptions) -> Result<WorkerGuard, Box<dyn std::error::Error>> {
    // initialize the file appender for logging
    let file_appender = tracing_appender::rolling::hourly(options.log_dir, "miru.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    // respect RUST_LOG environment variable if set, otherwise use provided log level
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(options.log_level.to_string()));

    if options.stdout {
        let subscriber = fmt()
            .with_env_filter(env_filter)
            .with_file(true)
            .with_line_number(true)
            .with_thread_ids(true)
            .with_thread_names(true);
        let _ = tracing::subscriber::set_global_default(subscriber.finish());
        Ok(guard)
    } else {
        let subscriber = fmt()
            .with_env_filter(env_filter)
            .with_writer(non_blocking)
            .with_file(true)
            .with_ansi(false)
            .with_line_number(true)
            .with_thread_ids(true)
            .with_thread_names(true);
        let _ = tracing::subscriber::set_global_default(subscriber.finish());
        Ok(guard)
    }
}
