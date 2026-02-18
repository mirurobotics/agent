// standard crates
use std::sync::{Arc, Mutex};

// internal crates
use miru_agent::http::{self, errors::HTTPErr, request::Params};
use openapi_client::models::{
    Deployment as BackendDeployment, DeploymentList, Device, Error as ApiError, ErrorResponse,
    TokenResponse,
};

// external crates
use axum::http::StatusCode;
use axum::Json;
use axum::Router;
use serde::Serialize;
use tokio::net::TcpListener;
use tokio::time::Duration;

// ================================ MOCK CALL ======================================= //

#[derive(Clone, Debug, PartialEq)]
pub enum MockCall {
    ActivateDevice,
    IssueDeviceToken,
    UpdateDevice,
    ListDeployments,
    GetDeployment,
    UpdateDeployment,
}

// ================================ CAPTURED REQUEST ================================ //

#[derive(Clone, Debug, PartialEq)]
pub struct CapturedRequest {
    pub method: reqwest::Method,
    pub url: String,
    pub query: Vec<(String, String)>,
    pub body: Option<String>,
    pub token: Option<String>,
}

// ================================ MOCK CLIENT ==================================== //

type ListDeploymentsFn = Mutex<Box<dyn Fn() -> Result<DeploymentList, HTTPErr> + Send + Sync>>;
type SingleDeploymentFn = Mutex<Box<dyn Fn() -> Result<BackendDeployment, HTTPErr> + Send + Sync>>;

pub struct MockClient {
    pub activate_device_fn: Box<dyn Fn() -> Result<Device, HTTPErr> + Send + Sync>,
    pub issue_device_token_fn: Box<dyn Fn() -> Result<TokenResponse, HTTPErr> + Send + Sync>,
    pub update_device_fn: Box<dyn Fn() -> Result<Device, HTTPErr> + Send + Sync>,
    pub list_deployments_fn: ListDeploymentsFn,
    pub get_deployment_fn: SingleDeploymentFn,
    pub update_deployment_fn: SingleDeploymentFn,
    pub calls: Arc<Mutex<Vec<MockCall>>>,
    pub requests: Arc<Mutex<Vec<CapturedRequest>>>,
}

impl Default for MockClient {
    fn default() -> Self {
        Self {
            activate_device_fn: Box::new(|| Ok(Device::default())),
            issue_device_token_fn: Box::new(|| Ok(TokenResponse::default())),
            update_device_fn: Box::new(|| Ok(Device::default())),
            list_deployments_fn: Mutex::new(Box::new(|| Ok(DeploymentList::default()))),
            get_deployment_fn: Mutex::new(Box::new(|| Ok(BackendDeployment::default()))),
            update_deployment_fn: Mutex::new(Box::new(|| Ok(BackendDeployment::default()))),
            calls: Arc::new(Mutex::new(Vec::new())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl MockClient {
    pub fn set_list_all_deployments<F>(&self, f: F)
    where
        F: Fn() -> Result<Vec<BackendDeployment>, HTTPErr> + Send + Sync + 'static,
    {
        *self.list_deployments_fn.lock().unwrap() = Box::new(move || {
            let data = f()?;
            Ok(DeploymentList {
                total_count: data.len() as i64,
                data,
                ..DeploymentList::default()
            })
        });
    }

    pub fn set_list_deployments_page<F>(&self, f: F)
    where
        F: Fn() -> Result<DeploymentList, HTTPErr> + Send + Sync + 'static,
    {
        *self.list_deployments_fn.lock().unwrap() = Box::new(f);
    }

    pub fn set_get_deployment<F>(&self, f: F)
    where
        F: Fn() -> Result<BackendDeployment, HTTPErr> + Send + Sync + 'static,
    {
        *self.get_deployment_fn.lock().unwrap() = Box::new(f);
    }

    pub fn set_update_deployment<F>(&self, f: F)
    where
        F: Fn() -> Result<BackendDeployment, HTTPErr> + Send + Sync + 'static,
    {
        *self.update_deployment_fn.lock().unwrap() = Box::new(f);
    }

    pub fn call_count(&self, target: MockCall) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| **call == target)
            .count()
    }

    pub fn num_update_device_calls(&self) -> usize {
        self.call_count(MockCall::UpdateDevice)
    }

    pub fn requests(&self) -> Vec<CapturedRequest> {
        self.requests.lock().unwrap().clone()
    }

    fn match_route(method: &reqwest::Method, path: &str) -> MockCall {
        use reqwest::Method;
        match (method, path) {
            (m, p) if *m == Method::POST && p.ends_with("/activate") => MockCall::ActivateDevice,
            (m, p) if *m == Method::POST && p.ends_with("/issue_token") => {
                MockCall::IssueDeviceToken
            }
            (m, p) if *m == Method::PATCH && p.starts_with("/devices/") => MockCall::UpdateDevice,
            (m, p) if *m == Method::GET && p == "/deployments" => MockCall::ListDeployments,
            (m, p) if *m == Method::GET && p.starts_with("/deployments/") => {
                MockCall::GetDeployment
            }
            (m, p) if *m == Method::PATCH && p.starts_with("/deployments/") => {
                MockCall::UpdateDeployment
            }
            _ => panic!("MockClient: unhandled route: {method} {path}"),
        }
    }

    fn handle_route(&self, call: &MockCall) -> Result<String, HTTPErr> {
        match call {
            MockCall::ActivateDevice => json(&(self.activate_device_fn)()?),
            MockCall::IssueDeviceToken => json(&(self.issue_device_token_fn)()?),
            MockCall::UpdateDevice => json(&(self.update_device_fn)()?),
            MockCall::ListDeployments => {
                let list = (self.list_deployments_fn.lock().unwrap())()?;
                json(&list)
            }
            MockCall::GetDeployment => json(&(self.get_deployment_fn.lock().unwrap())()?),
            MockCall::UpdateDeployment => json(&(self.update_deployment_fn.lock().unwrap())()?),
        }
    }

    fn dispatch(&self, method: &reqwest::Method, url: &str) -> Result<String, HTTPErr> {
        let path = url
            .strip_prefix(http::ClientI::base_url(self))
            .unwrap_or(url);
        let path = path.split('?').next().unwrap_or(path);
        let call = Self::match_route(method, path);
        self.calls.lock().unwrap().push(call.clone());
        self.handle_route(&call)
    }
}

fn json<T: Serialize>(val: &T) -> Result<String, HTTPErr> {
    Ok(serde_json::to_string(val).unwrap())
}

impl http::ClientI for MockClient {
    fn base_url(&self) -> &str {
        "http://mock"
    }

    async fn execute(
        &self,
        params: Params<'_>,
    ) -> Result<(String, miru_agent::http::request::Meta), HTTPErr> {
        let meta = params.meta()?;
        self.requests.lock().unwrap().push(CapturedRequest {
            method: params.method.clone(),
            url: params.url.to_string(),
            query: params.query.clone(),
            body: params.body.clone(),
            token: params.token.map(|t| t.to_string()),
        });
        let text = self.dispatch(&params.method, params.url)?;
        Ok((text, meta))
    }
}

// ================================ MOCK SERVER ==================================== //
pub struct Server {
    pub base_url: String,
    _handle: tokio::task::JoinHandle<()>,
}

pub async fn run_server(router: Router) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Server {
        base_url,
        _handle: handle,
    }
}

pub async fn ok() -> &'static str {
    "ok"
}

pub async fn hello() -> &'static str {
    "hello"
}

pub async fn empty() -> &'static str {
    ""
}

pub async fn echo(body: String) -> String {
    body
}

pub async fn json_response() -> Json<serde_json::Value> {
    Json(serde_json::json!({"name": "alice", "age": 30}))
}

pub async fn not_found() -> StatusCode {
    StatusCode::NOT_FOUND
}

pub async fn unauthorized() -> (StatusCode, Json<ErrorResponse>) {
    let body = ErrorResponse::new(ApiError::new(
        "invalid_jwt_auth".to_string(),
        std::collections::HashMap::new(),
        "invalid token".to_string(),
    ));
    (StatusCode::UNAUTHORIZED, Json(body))
}

pub async fn internal_server_error() -> StatusCode {
    StatusCode::INTERNAL_SERVER_ERROR
}

pub async fn bad_json() -> &'static str {
    "not valid json{"
}

pub async fn slow() -> &'static str {
    tokio::time::sleep(Duration::from_secs(5)).await;
    "done"
}
