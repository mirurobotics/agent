// standard crates
use std::fmt;

// internal crates
use crate::errors::{Code, HTTPCode, Error, Trace};

#[derive(Debug)]
pub struct DateTimeParseErr {
    pub source: chrono::ParseError,
    pub trace: Box<Trace>,
}

impl Error for DateTimeParseErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for DateTimeParseErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Failed to parse date time: {}", self.source)
    }
}

#[derive(Debug)]
pub enum ModelsErr {
    DateTimeParseErr(Box<DateTimeParseErr>),
}

macro_rules! forward_error_method {
    ($self:ident, $method:ident $(, $arg:expr)?) => {
        match $self {
            ModelsErr::DateTimeParseErr(e) => e.$method($($arg)?),
        }
    };
}

impl fmt::Display for ModelsErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        forward_error_method!(self, fmt, f)
    }
}

impl Error for ModelsErr {
    fn code(&self) -> Code {
        forward_error_method!(self, code)
    }

    fn http_status(&self) -> HTTPCode {
        forward_error_method!(self, http_status)
    }

    fn is_network_connection_error(&self) -> bool {
        forward_error_method!(self, is_network_connection_error)
    }

    fn params(&self) -> Option<serde_json::Value> {
        forward_error_method!(self, params)
    }
}
