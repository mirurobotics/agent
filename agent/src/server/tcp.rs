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
use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tracing::{info, warn};

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
    tokio::task::spawn(async move {
        let port = listener
            .local_addr()
            .map_err(|e| {
                ServerErr::RunAxumServerErr(RunAxumServerErr {
                    source: e,
                    trace: trace!(),
                })
            })?
            .port();
        let app = routes::app(state).layer(middleware::from_fn(move |req, next| {
            check_host(port, req, next)
        }));
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

/// Reject requests whose host isn't this listener's loopback address, so a web
/// page served under another hostname can't reach the API through DNS
/// rebinding.
async fn check_host(port: u16, req: Request, next: Next) -> Response {
    let host = req
        .uri()
        .authority()
        .map(|authority| authority.as_str())
        .or_else(|| {
            req.headers()
                .get(header::HOST)
                .and_then(|value| value.to_str().ok())
        });
    if host.is_some_and(|host| is_loopback_host(host, port)) {
        return next.run(req).await;
    }
    warn!("Rejected local device API request with host {host:?}");
    StatusCode::FORBIDDEN.into_response()
}

fn is_loopback_host(host: &str, port: u16) -> bool {
    let Some((name, host_port)) = host.rsplit_once(':') else {
        return false;
    };
    host_port.parse() == Ok(port) && (name == "127.0.0.1" || name.eq_ignore_ascii_case("localhost"))
}
