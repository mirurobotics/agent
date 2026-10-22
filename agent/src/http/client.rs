// standard crates
use std::sync::Arc;

// internal crates
use crate::clock::Clock;
use crate::http::{
    date,
    errors::{reqwest_err_to_http_client_err, BuildReqwestErr, HTTPErr, TimeoutErr},
    request, response,
};
use crate::trace;

// external crates
use serde::de::DeserializeOwned;
use tokio::time::timeout;
use tracing::debug;

#[derive(Debug)]
pub struct Client {
    client: reqwest::Client,
    base_url: String,
    /// The URL whose responses may set the clock: `base_url`, parsed. Only a
    /// response from this URL's origin has its `Date` header recorded.
    clock_url: Option<reqwest::Url>,
    headers: request::Headers,
    clock: Clock,
}

// Per the reqwest docs, we do not need to wrap the client in Rc or Arc to reuse it
// in a thread safe manner since reqwest already handles this under the hood [1]. Thus,
// our job is easy: just initialize the client and clone when wanting to reuse it [2].
//
// Sources:
// 1. https://docs.rs/reqwest/latest/reqwest/struct.Client.html
// 2. https://users.rust-lang.org/t/reqwest-http-client-fails-when-too-much-concurrency/55644

pub trait ClientI: Send + Sync {
    fn base_url(&self) -> &str;

    /// The clock this client feeds with the server's time.
    fn clock(&self) -> &Clock;

    /// Build, send, handle response — returns body text and request metadata.
    fn execute(
        &self,
        params: request::Params<'_>,
    ) -> impl std::future::Future<Output = Result<(String, request::Meta), HTTPErr>> + Send;
}

impl ClientI for Client {
    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn clock(&self) -> &Clock {
        &self.clock
    }

    async fn execute(
        &self,
        params: request::Params<'_>,
    ) -> Result<(String, request::Meta), HTTPErr> {
        let req = self.build_request(params)?;
        let meta = req.meta.clone();
        let resp = self.send(req).await?;
        let text = response::handle(resp).await?;
        Ok((text, meta))
    }
}

impl<T: ClientI> ClientI for Arc<T> {
    fn base_url(&self) -> &str {
        self.as_ref().base_url()
    }

    fn clock(&self) -> &Clock {
        self.as_ref().clock()
    }

    async fn execute(
        &self,
        params: request::Params<'_>,
    ) -> Result<(String, request::Meta), HTTPErr> {
        self.as_ref().execute(params).await
    }
}

impl Client {
    pub fn new(base_url: &str) -> Result<Self, HTTPErr> {
        Self::with_clock(base_url, Clock::new())
    }

    /// A client that records the server's time into `clock` from every
    /// response that comes from `base_url`'s origin.
    pub fn with_clock(base_url: &str, clock: Clock) -> Result<Self, HTTPErr> {
        let client = reqwest::Client::builder().build().map_err(|e| {
            HTTPErr::BuildReqwestErr(BuildReqwestErr {
                source: e,
                trace: trace!(),
            })
        })?;
        Ok(Client {
            client,
            base_url: base_url.to_string(),
            clock_url: reqwest::Url::parse(base_url).ok(),
            headers: request::Headers::default(),
            clock,
        })
    }

    pub fn build_request(&self, params: request::Params) -> Result<request::Request, HTTPErr> {
        request::build(&self.client, &self.headers, params)
    }

    pub async fn send(&self, req: request::Request) -> Result<response::Response, HTTPErr> {
        match timeout(req.meta.timeout, self.client.execute(req.reqwest)).await {
            Err(e) => Err(HTTPErr::TimeoutErr(TimeoutErr {
                msg: e.to_string(),
                request: req.meta,
                trace: trace!(),
            })),
            Ok(Err(e)) => Err(reqwest_err_to_http_client_err(e, req.meta, trace!())),
            Ok(Ok(response)) => {
                self.observe_server_time(&response);
                Ok(response::Response {
                    reqwest: response,
                    meta: req.meta,
                })
            }
        }
    }

    /// Records the response's `Date` header (and `Age`, if a cache served it),
    /// error statuses included, but only when the response came from
    /// `clock_url`'s origin. A request to any other host, or a redirect to one,
    /// must not set the agent's time.
    fn observe_server_time(&self, response: &reqwest::Response) {
        if !self.is_clock_source(response.url()) {
            return;
        }
        let headers = response.headers();
        let header = |name| headers.get(name).and_then(|v| v.to_str().ok());
        let Some(date) = header(reqwest::header::DATE) else {
            return;
        };
        match date::server_time(date, header(reqwest::header::AGE)) {
            Some(server_time) => self.clock.observe_server_time(server_time),
            None => debug!("http: ignoring unparseable Date header {date:?}"),
        }
    }

    /// Whether `url` has the same scheme, host and port as `clock_url`.
    fn is_clock_source(&self, url: &reqwest::Url) -> bool {
        self.clock_url
            .as_ref()
            .is_some_and(|clock_url| clock_url.origin() == url.origin())
    }
}

pub async fn fetch<T>(client: &impl ClientI, params: request::Params<'_>) -> Result<T, HTTPErr>
where
    T: DeserializeOwned,
{
    let (text, meta) = client.execute(params).await?;
    response::parse_json(text, meta)
}
