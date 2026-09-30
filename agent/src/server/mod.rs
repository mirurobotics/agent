// internal crates
use crate::filesys;

pub mod errors;
pub mod handlers;
pub mod response;
pub mod routes;
pub mod sse;
pub mod state;
pub mod tcp;
#[cfg(unix)]
pub mod unix;

pub use self::errors::ServerErr;
pub use self::state::State;

#[derive(Debug)]
pub struct Options {
    /// Unix socket path (unix only).
    pub socket_file: filesys::File,
    /// Loopback TCP port; `None` disables the TCP transport, `Some(0)` lets
    /// the OS assign a free port.
    pub tcp_port: Option<u16>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            socket_file: filesys::File::new("/run/miru/miru.sock"),
            tcp_port: None,
        }
    }
}
