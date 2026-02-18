// standard library
use std::sync::Arc;

// internal crates
use crate::errors::Error;
use crate::http::{
    errors::{reqwest_err_to_http_client_err, HTTPErr},
    errors::{CacheErr, TimeoutErr},
    request, response,
};
use crate::trace;

// external crates
use moka::future::Cache;
use tokio::sync::OnceCell;
use tokio::time::{sleep, timeout, Duration};
use uuid::Uuid;

// type aliases
type RequestKey = String;
type Response = String;
type RequestID = Uuid;
type IsCacheHit = bool;

#[derive(Debug)]
pub struct Client {
    client: reqwest::Client,
    base_url: String,
    default_timeout: Duration,
    headers: request::Headers,
    cache: Cache<RequestKey, (Response, RequestID)>,
}

// Use Lazy to implement the Singleton(ish) Pattern for the reqwest client (see the
// README for more information). Per the documentation, we do not need to wrap the
// client in Rc or Arc to reuse it in a thread safe manner since reqwest already handles
// this under the hood [1]. Thus, our job is easy: just initialize the client and clone
// when wanting to reuse it [2]. One last note, avoid the reqwest::Client::new() method
// since it panics on a failure. Instead, use the reqwest::Client::builder() method so
// we can handle the failure gracefully [3].

// Sources:
// 1. https://docs.rs/reqwest/latest/reqwest/struct.Client.html
// 2. https://users.rust-lang.org/t/reqwest-http-client-fails-when-too-much-concurrency/55644
// 3. https://docs.rs/reqwest/latest/reqwest/struct.Client.html#method.builder
static CLIENT: OnceCell<reqwest::Client> = OnceCell::const_new();

async fn init_client() -> reqwest::Client {
    loop {
        let client = reqwest::Client::builder().build();
        if let Ok(client) = client {
            return client;
        }
        // wait 60 seconds before trying again
        sleep(Duration::from_secs(60)).await;
    }
}

pub trait ClientI: Send + Sync {
    fn base_url(&self) -> &str;
    fn default_timeout(&self) -> Duration;

    /// Build, send, handle response — returns body text.
    fn execute(
        &self,
        params: request::Params<'_>,
    ) -> impl std::future::Future<Output = Result<String, HTTPErr>> + Send;

    /// Same as execute but with response caching.
    fn execute_cached(
        &self,
        params: request::Params<'_>,
    ) -> impl std::future::Future<Output = Result<String, HTTPErr>> + Send;
}

impl ClientI for Client {
    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn default_timeout(&self) -> Duration {
        self.default_timeout
    }

    async fn execute(&self, params: request::Params<'_>) -> Result<String, HTTPErr> {
        let meta = params.meta();
        let request = self.build_request(params)?;
        let (http_resp, meta) = self.send(meta, request).await?;
        let text = response::handle(http_resp, meta).await?;
        Ok(text)
    }

    async fn execute_cached(&self, params: request::Params<'_>) -> Result<String, HTTPErr> {
        let key = params.url_with_query();
        let meta = params.meta();
        let request = self.build_request(params)?;
        let (text, _is_cache_hit) = self.send_cached(meta, key, request).await?;
        Ok(text)
    }
}

impl<T: ClientI> ClientI for Arc<T> {
    fn base_url(&self) -> &str {
        self.as_ref().base_url()
    }

    fn default_timeout(&self) -> Duration {
        self.as_ref().default_timeout()
    }

    async fn execute(&self, params: request::Params<'_>) -> Result<String, HTTPErr> {
        self.as_ref().execute(params).await
    }

    async fn execute_cached(&self, params: request::Params<'_>) -> Result<String, HTTPErr> {
        self.as_ref().execute_cached(params).await
    }
}

impl Client {
    pub async fn new(base_url: &str) -> Self {
        let client = CLIENT.get_or_init(init_client).await;

        Client {
            client: client.clone(),
            base_url: base_url.to_string(),
            default_timeout: Duration::from_secs(10),
            headers: request::Headers::default(),
            cache: Cache::builder()
                .time_to_live(Duration::from_secs(2))
                .build(),
        }
    }

    pub fn build_request(&self, params: request::Params) -> Result<reqwest::Request, HTTPErr> {
        request::build(&self.client, &self.headers, params)
    }

    pub async fn send(
        &self,
        meta: request::Meta,
        request: reqwest::Request,
    ) -> Result<(reqwest::Response, request::Meta), HTTPErr> {
        let time_limit = match request.timeout() {
            Some(time_limit) => *time_limit,
            None => self.default_timeout,
        };
        match timeout(time_limit, self.client.execute(request)).await {
            Err(e) => Err(HTTPErr::TimeoutErr(TimeoutErr {
                msg: e.to_string(),
                request: meta,
                trace: trace!(),
            })),
            Ok(Err(e)) => Err(reqwest_err_to_http_client_err(e, meta, trace!())),
            Ok(Ok(response)) => Ok((response, meta)),
        }
    }

    pub async fn send_cached(
        &self,
        meta: request::Meta,
        key: RequestKey,
        request: reqwest::Request,
    ) -> Result<(String, IsCacheHit), HTTPErr> {
        let id = Uuid::new_v4();

        let result = self
            .cache
            .try_get_with(key, async move {
                let (response, meta) = self.send(meta, request).await?;
                Ok((response::handle(response, meta).await?, id))
            })
            .await
            .map_err(|e: Arc<HTTPErr>| {
                HTTPErr::CacheErr(CacheErr {
                    code: e.code(),
                    http_status: e.http_status(),
                    is_network_connection_error: e.is_network_connection_error(),
                    params: e.params(),
                    msg: e.to_string(),
                    trace: trace!(),
                })
            })?;
        let is_cache_hit = result.1 != id;
        Ok((result.0, is_cache_hit))
    }

    pub fn new_with(
        base_url: &str,
        default_timeout: Duration,
        cache: Cache<String, (String, Uuid)>,
    ) -> Self {
        Client {
            client: reqwest::Client::new(),
            base_url: base_url.to_string(),
            default_timeout,
            headers: request::Headers::default(),
            cache,
        }
    }
}
