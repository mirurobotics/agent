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

/// Loopback port used when the TCP server is enabled and no other port is set.
/// 6478 is "MIRU" on a phone keypad (M=6, I=4, R=7, U=8).
pub const DEFAULT_TCP_PORT: u16 = 6478;

#[derive(Debug)]
pub struct Options {
    /// Unix socket path (unix only).
    pub socket_file: filesys::File,
    /// Loopback TCP port used when the TCP server is enabled. `0` lets the OS
    /// assign a free port.
    pub tcp_port: u16,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            socket_file: filesys::File::new("/run/miru/miru.sock"),
            tcp_port: DEFAULT_TCP_PORT,
        }
    }
}
