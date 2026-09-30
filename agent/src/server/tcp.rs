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
        let app = routes::router(state).layer(middleware::from_fn(move |req, next| {
            check_loopback(port, req, next)
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

/// Reject requests that aren't addressed to this listener's loopback URL.
/// The host (URI authority, else the Host header) must be `127.0.0.1:<port>`
/// or `localhost:<port>`, which stops DNS rebinding. An Origin header, when
/// present, must be that same loopback URL, which stops a cross-site page
/// from sending a request the browser would give a loopback Host.
async fn check_loopback(port: u16, req: Request, next: Next) -> Response {
    let host = extract_host(&req);
    if !host_allowed(host, port) {
        warn!("Rejected local device API request with host {host:?}");
        return StatusCode::FORBIDDEN.into_response();
    }
    let origin = extract_origin(&req);
    if !origin_allowed(origin, port) {
        warn!("Rejected local device API request with origin {origin:?}");
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(req).await
}

fn extract_host(req: &Request) -> Option<&str> {
    req.uri()
        .authority()
        .map(|authority| authority.as_str())
        .or_else(|| {
            req.headers()
                .get(header::HOST)
                .and_then(|value| value.to_str().ok())
        })
}

fn host_allowed(host: Option<&str>, port: u16) -> bool {
    host.is_some_and(|host| is_loopback_host(host, port))
}

fn extract_origin(req: &Request) -> Option<&str> {
    req.headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
}

/// A missing Origin is allowed. A present Origin must be this listener's loopback URL.
fn origin_allowed(origin: Option<&str>, port: u16) -> bool {
    match origin {
        None => true,
        Some(origin) => origin
            .strip_prefix("http://")
            .is_some_and(|host| is_loopback_host(host, port)),
    }
}

fn is_loopback_host(host: &str, port: u16) -> bool {
    let Some((name, host_port)) = host.rsplit_once(':') else {
        return false;
    };
    host_port.parse() == Ok(port) && (name == "127.0.0.1" || name.eq_ignore_ascii_case("localhost"))
}
