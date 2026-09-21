// standard crates
use std::env;
use std::future::Future;
use std::os::unix::io::{FromRawFd, RawFd};
use std::sync::Arc;

// internal crates
use crate::filesys::{self, files, PathExt};
use crate::server::{
    errors::{BindUnixSocketErr, RunAxumServerErr, ServerErr},
    routes::{self, Options},
    State,
};
use crate::trace;

// external crates
use tokio::net::UnixListener;
use tokio::task::JoinHandle;

pub(crate) async fn serve(
    options: &Options,
    state: Arc<State>,
    shutdown_signal: impl Future<Output = ()> + Send + 'static,
) -> Result<JoinHandle<Result<(), ServerErr>>, ServerErr> {
    let app = routes::app(state);

    // obtain the unix socket file listener
    let listener = acquire_unix_socket_listener(&options.socket_file, async move {
        create_unix_socket_listener(&options.socket_file).await
    })
    .await?;

    // serve with graceful shutdown
    let server_handle = tokio::task::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal)
            .await
            .map_err(|e| {
                ServerErr::RunAxumServerErr(RunAxumServerErr {
                    source: e,
                    trace: trace!(),
                })
            })
    });

    Ok(server_handle)
}

async fn acquire_unix_socket_listener(
    socket_file: &filesys::File,
    fallback: impl Future<Output = Result<UnixListener, ServerErr>>,
) -> Result<UnixListener, ServerErr> {
    let listener = if let Ok(listen_fds) = env::var("LISTEN_FDS") {
        let listen_fds = listen_fds.parse::<u32>().map_err(|e| {
            ServerErr::BindUnixSocketErr(BindUnixSocketErr {
                socket_file: socket_file.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Failed to parse LISTEN_FDS: {e}"),
                ),
                trace: trace!(),
            })
        })?;
        if listen_fds >= 1 {
            // FD#3 is the first one
            let fd: RawFd = 3;
            // SAFETY: fd=3 was handed to us by systemd
            let std_listener = unsafe { std::os::unix::net::UnixListener::from_raw_fd(fd) };
            std_listener.set_nonblocking(true).map_err(|e| {
                ServerErr::BindUnixSocketErr(BindUnixSocketErr {
                    socket_file: socket_file.clone(),
                    source: e,
                    trace: trace!(),
                })
            })?;
            UnixListener::from_std(std_listener).map_err(|e| {
                ServerErr::BindUnixSocketErr(BindUnixSocketErr {
                    socket_file: socket_file.clone(),
                    source: e,
                    trace: trace!(),
                })
            })?
        } else {
            fallback.await?
        }
    } else {
        fallback.await?
    };
    Ok(listener)
}

async fn create_unix_socket_listener(
    socket_file: &filesys::File,
) -> Result<UnixListener, ServerErr> {
    files::delete(socket_file).await?;
    let socket_path = socket_file.path();
    tokio::net::UnixListener::bind(socket_path).map_err(|e| {
        ServerErr::BindUnixSocketErr(BindUnixSocketErr {
            socket_file: socket_file.clone(),
            source: e,
            trace: trace!(),
        })
    })
}
