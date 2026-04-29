// standard crates
use std::fmt::Display;
use std::path::PathBuf;

// external crates
use serde::{Deserialize, Serialize};
use thiserror::Error;
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, prelude::*, registry::Registry, reload, EnvFilter};

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

pub struct Options {
    pub stdout: bool,
    pub log_level: LogLevel,
    pub log_dir: PathBuf,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            stdout: true,
            log_level: LogLevel::Info,
            log_dir: PathBuf::from("/var/log/miru"),
        }
    }
}

#[derive(Debug, Error)]
pub enum LogsErr {
    #[error("failed to install global tracing subscriber: {0}")]
    SetGlobalDefault(#[from] tracing::subscriber::SetGlobalDefaultError),
    #[error("failed to reload tracing filter: {0}")]
    ReloadFailed(String),
}

impl crate::errors::Error for LogsErr {}

type ReloadHandle = reload::Handle<EnvFilter, Registry>;

pub struct LoggingGuard {
    _worker: WorkerGuard,
    reload_handle: ReloadHandle,
    // True if RUST_LOG provided the initial filter; reload_level becomes a no-op.
    env_filter_locked: bool,
}

impl LoggingGuard {
    /// Reload the active log-level filter.
    ///
    /// If `RUST_LOG` was set at process startup, this is a no-op; the env filter wins.
    /// Adjusts filter/level only — does not change the log destination.
    pub fn reload_level(&self, level: LogLevel) -> Result<(), LogsErr> {
        if self.env_filter_locked {
            return Ok(());
        }
        let new_filter = EnvFilter::new(level.to_string());
        self.reload_handle
            .reload(new_filter)
            .map_err(|e| LogsErr::ReloadFailed(e.to_string()))?;
        Ok(())
    }

    pub fn env_filter_locked(&self) -> bool {
        self.env_filter_locked
    }
}

pub fn init(options: Options) -> Result<LoggingGuard, LogsErr> {
    // initialize the file appender for logging
    let file_appender = tracing_appender::rolling::hourly(options.log_dir, "miru.log");
    let (non_blocking, worker_guard) = tracing_appender::non_blocking(file_appender);

    // respect RUST_LOG environment variable if set, otherwise use provided log level
    let (env_filter, env_filter_locked) = match EnvFilter::try_from_default_env() {
        Ok(f) => (f, true),
        Err(_) => (EnvFilter::new(options.log_level.to_string()), false),
    };

    let (reload_layer, reload_handle) = reload::Layer::new(env_filter);

    if options.stdout {
        let fmt_layer = fmt::layer()
            .with_file(true)
            .with_line_number(true)
            .with_thread_ids(true)
            .with_thread_names(true);
        let subscriber = Registry::default().with(reload_layer).with(fmt_layer);
        tracing::subscriber::set_global_default(subscriber)?;
    } else {
        let fmt_layer = fmt::layer()
            .with_writer(non_blocking)
            .with_file(true)
            .with_ansi(false)
            .with_line_number(true)
            .with_thread_ids(true)
            .with_thread_names(true);
        let subscriber = Registry::default().with(reload_layer).with(fmt_layer);
        tracing::subscriber::set_global_default(subscriber)?;
    }

    Ok(LoggingGuard {
        _worker: worker_guard,
        reload_handle,
        env_filter_locked,
    })
}
