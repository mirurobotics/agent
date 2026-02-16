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

pub trait Error: std::error::Error {
    fn code(&self) -> Code {
        Code::InternalServerError
    }
    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }
    fn params(&self) -> Option<serde_json::Value> {
        None
    }
    fn is_network_connection_error(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone)]
pub struct Trace {
    pub file: &'static str,
    pub line: u32,
}

#[macro_export]
macro_rules! trace {
    () => {
        Box::new($crate::errors::Trace {
            file: file!(),
            line: line!(),
        })
    };
}

#[macro_export]
macro_rules! impl_error {
    ($enum_name:ident { $($variant:ident),+ $(,)? }) => {
        impl $crate::errors::Error for $enum_name {
            fn code(&self) -> $crate::errors::Code {
                match self {
                    $(Self::$variant(e) => e.code(),)+
                }
            }
            fn http_status(&self) -> $crate::errors::HTTPCode {
                match self {
                    $(Self::$variant(e) => e.http_status(),)+
                }
            }
            fn is_network_connection_error(&self) -> bool {
                match self {
                    $(Self::$variant(e) => e.is_network_connection_error(),)+
                }
            }
            fn params(&self) -> Option<serde_json::Value> {
                match self {
                    $(Self::$variant(e) => e.params(),)+
                }
            }
        }
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
