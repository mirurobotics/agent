// standard library
use std::fmt;

// external crates
use axum::http::StatusCode;
#[allow(unused_imports)]
use tracing::{error, info, trace, warn};

pub type HTTPCode = StatusCode;

#[derive(Debug, Clone)]
pub enum Code {
    InternalServerError,
    ResourceNotFound,
    BackendError(String),
}

impl Code {
    pub fn as_str(&self) -> &str {
        match self {
            Self::InternalServerError => "internal_server_error",
            Self::ResourceNotFound => "resource_not_found",
            Self::BackendError(code) => code,
        }
    }
}

pub trait MiruError
where
    Self: fmt::Debug + fmt::Display,
{
    fn code(&self) -> Code;
    fn http_status(&self) -> HTTPCode;
    fn params(&self) -> Option<serde_json::Value>;
    fn is_network_connection_error(&self) -> bool;
}

pub fn are_all_network_connection_errors<I>(errors: I) -> bool
where
    I: IntoIterator,
    I::Item: AsRef<dyn MiruError>,
{
    errors
        .into_iter()
        .all(|e| e.as_ref().is_network_connection_error())
}

#[derive(Debug, Clone)]
pub struct Trace {
    pub file: &'static str,
    pub line: u32,
    // pub stack_trace: Backtrace,
}

#[macro_export]
macro_rules! trace {
    () => {
        Box::new($crate::errors::Trace {
            file: file!(),
            line: line!(),
            // stack_trace: backtrace::Backtrace::new(),
        })
    };
}

#[macro_export]
macro_rules! deserialize_error {
    ($struct_name:expr, $field_name:expr, $default:expr) => {{
        ::tracing::error!(
            "'{}' missing from struct '{}', setting to default: '{:?}'",
            $field_name,
            $struct_name,
            $default
        );
        $default
    }};
}

#[macro_export]
macro_rules! deserialize_warn {
    ($struct_name:expr, $field_name:expr, $default:expr) => {{
        ::tracing::warn!(
            "'{}' missing from struct '{}', setting to default: '{:?}'",
            $field_name,
            $struct_name,
            $default
        );
        $default
    }};
}
