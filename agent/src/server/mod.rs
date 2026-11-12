// internal crates
#[cfg(unix)]
use crate::filesys;

pub mod auth;
pub mod errors;
pub mod handlers;
pub mod request;
pub mod response;
pub mod routes;
pub mod sse;
pub mod state;
pub mod tcp;
#[cfg(unix)]
pub mod unix;

pub use self::errors::ServerErr;
pub use self::state::State;

/// Whether the TCP server is on when settings don't say. It is on for Windows,
/// where it is the only transport, and off elsewhere, where the Unix socket
/// restricts access to the `miru` group. TCP requests must carry the bearer
/// token the agent writes to the discovery file.
pub const DEFAULT_ENABLE_TCP_SERVER: bool = cfg!(windows);

/// Whether the Unix socket server is on when settings don't say. Windows
/// has no Unix socket, so it is off there.
pub const DEFAULT_ENABLE_SOCKET_SERVER: bool = cfg!(unix);

/// Loopback port used when the TCP server is enabled and no other port is set.
/// 6478 is "MIRU" on a phone keypad (M=6, I=4, R=7, U=8).
pub const DEFAULT_TCP_PORT: u16 = 6478;

#[derive(Debug)]
pub struct Options {
    /// Unix socket path.
    #[cfg(unix)]
    pub socket_file: filesys::File,
    /// Loopback TCP port used when the TCP server is enabled. `0` lets the OS
    /// assign a free port.
    pub tcp_port: u16,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            #[cfg(unix)]
            socket_file: filesys::File::new("/run/miru/miru.sock"),
            tcp_port: DEFAULT_TCP_PORT,
        }
    }
}
