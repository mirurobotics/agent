// standard crates
use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

// internal crates
use crate::filesys;
use crate::server::{
    auth::{check_bearer, BearerToken},
    discovery,
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
    Router,
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

/// Serve the local device API on `listener`. Every request must pass the
/// loopback check (403) and then carry the bearer token (401). The port and a
/// freshly generated token are written to `discovery_file` before serving, and
/// the file is removed after graceful shutdown. A token or discovery failure
/// returns an error without serving.
pub async fn serve(
    listener: TcpListener,
    state: Arc<State>,
    discovery_file: filesys::File,
    shutdown_signal: impl Future<Output = ()> + Send + 'static,
) -> Result<JoinHandle<Result<(), ServerErr>>, ServerErr> {
    let port = listener
        .local_addr()
        .map_err(|e| {
            ServerErr::RunAxumServerErr(RunAxumServerErr {
                source: e,
                trace: trace!(),
            })
        })?
        .port();
    let token = BearerToken::generate()?;
    discovery::write(&discovery_file, port, &token).await?;
    let router = router(state, port, Arc::new(token));
    Ok(tokio::task::spawn(run(
        listener,
        router,
        shutdown_signal,
        discovery_file,
    )))
}

/// The shared router behind the bearer check, behind the loopback check.
fn router(state: Arc<State>, port: u16, token: Arc<BearerToken>) -> Router {
    routes::router(state)
        .layer(middleware::from_fn(move |req, next| {
            check_bearer(token.clone(), req, next)
        }))
        .layer(middleware::from_fn(move |req, next| {
            check_loopback(port, req, next)
        }))
}

async fn run(
    listener: TcpListener,
    router: Router,
    shutdown_signal: impl Future<Output = ()> + Send + 'static,
    discovery_file: filesys::File,
) -> Result<(), ServerErr> {
    let result = axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal)
        .await
        .map_err(|e| {
            ServerErr::RunAxumServerErr(RunAxumServerErr {
                source: e,
                trace: trace!(),
            })
        });
    if let Err(e) = discovery::remove(&discovery_file).await {
        warn!("Failed to remove discovery file: {e}");
    }
    result
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

#[cfg(test)]
mod tests {
    // internal crates
    use super::*;

    // external crates
    use axum::body::Body;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    const PORT: u16 = 6478;

    fn request(uri: &str, host: Option<&str>, origin: Option<&str>) -> Request {
        let mut builder = Request::builder().uri(uri);
        if let Some(host) = host {
            builder = builder.header(header::HOST, host);
        }
        if let Some(origin) = origin {
            builder = builder.header(header::ORIGIN, origin);
        }
        builder.body(Body::empty()).unwrap()
    }

    async fn status(request: Request) -> StatusCode {
        let app = Router::new()
            .route("/health", get(|| async { StatusCode::OK }))
            .layer(middleware::from_fn(move |req, next| {
                check_loopback(PORT, req, next)
            }));
        app.oneshot(request).await.unwrap().status()
    }

    #[tokio::test]
    async fn allows_localhost_host() {
        let status = status(request("/health", Some("localhost:6478"), None)).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn allows_loopback_host() {
        let status = status(request("/health", Some("127.0.0.1:6478"), None)).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn rejects_foreign_host() {
        let status = status(request("/health", Some("attacker.example:6478"), None)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn rejects_loopback_host_on_another_port() {
        let status = status(request("/health", Some("127.0.0.1:6479"), None)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn rejects_host_without_port() {
        let status = status(request("/health", Some("localhost"), None)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn rejects_missing_host() {
        let status = status(request("/health", None, None)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn authority_wins_over_host_header() {
        let status = status(request(
            "http://attacker.example:6478/health",
            Some("127.0.0.1:6478"),
            None,
        ))
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn allows_loopback_origin() {
        let status = status(request(
            "/health",
            Some("127.0.0.1:6478"),
            Some("http://127.0.0.1:6478"),
        ))
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn allows_localhost_origin() {
        let status = status(request(
            "/health",
            Some("127.0.0.1:6478"),
            Some("http://localhost:6478"),
        ))
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn rejects_foreign_origin() {
        let status = status(request(
            "/health",
            Some("127.0.0.1:6478"),
            Some("https://attacker.example"),
        ))
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn rejects_null_origin() {
        let status = status(request("/health", Some("127.0.0.1:6478"), Some("null"))).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn rejects_loopback_origin_on_another_port() {
        let status = status(request(
            "/health",
            Some("127.0.0.1:6478"),
            Some("http://127.0.0.1:6479"),
        ))
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
}
