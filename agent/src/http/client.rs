// standard library
use std::sync::Arc;

// internal crates
use crate::http::{
    errors::{reqwest_err_to_http_client_err, BuildReqwestErr, HTTPErr, TimeoutErr},
    request, response,
};
use crate::trace;
use serde::de::DeserializeOwned;

// external crates
use tokio::time::timeout;

#[derive(Debug)]
pub struct Client {
    client: reqwest::Client,
    base_url: String,
    headers: request::Headers,
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

    async fn execute(
        &self,
        params: request::Params<'_>,
    ) -> Result<(String, request::Meta), HTTPErr> {
        self.as_ref().execute(params).await
    }
}

impl Client {
    pub fn new(base_url: &str) -> Result<Self, HTTPErr> {
        let client = reqwest::Client::builder().build().map_err(|e| {
            HTTPErr::BuildReqwestErr(BuildReqwestErr {
                source: e,
                trace: trace!(),
            })
        })?;
        Ok(Client {
            client,
            base_url: base_url.to_string(),
            headers: request::Headers::default(),
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
            Ok(Ok(response)) => Ok(response::Response {
                reqwest: response,
                meta: req.meta,
            }),
        }
    }
}

pub async fn fetch<T>(client: &impl ClientI, params: request::Params<'_>) -> Result<T, HTTPErr>
where
    T: DeserializeOwned,
{
    let (text, meta) = client.execute(params).await?;
    response::parse_json(text, meta)
}
