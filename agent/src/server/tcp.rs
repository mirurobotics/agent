// standard crates
use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

// internal crates
use crate::server::{
    errors::{BindTcpListenerErr, RunAxumServerErr, ServerErr},
    routes, State,
};
use crate::trace;

// external crates
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tracing::info;

/// Bind a listener on the IPv4 loopback interface only, so the local device
/// API is never reachable from the network. Port `0` lets the OS pick a free
/// port; read the bound port back with `TcpListener::local_addr`.
pub async fn bind(port: u16) -> Result<TcpListener, ServerErr> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let listener = TcpListener::bind(addr).await.map_err(|e| {
        ServerErr::BindTcpListenerErr(BindTcpListenerErr {
            addr,
            source: e,
            trace: trace!(),
        })
    })?;
    let bound = listener.local_addr().unwrap_or(addr);
    info!("Local device API listening on http://{bound}");
    Ok(listener)
}

pub fn serve(
    listener: TcpListener,
    state: Arc<State>,
    shutdown_signal: impl Future<Output = ()> + Send + 'static,
) -> JoinHandle<Result<(), ServerErr>> {
    let app = routes::app(state);
    tokio::task::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal)
            .await
            .map_err(|e| {
                ServerErr::RunAxumServerErr(RunAxumServerErr {
                    source: e,
                    trace: trace!(),
                })
            })
    })
}
