// standard crates
use std::sync::{Arc, Mutex};

// internal crates
use backend_api::models::{
    Deployment as BackendDeployment, DeploymentList, Device, Error as ApiError, ErrorResponse,
    GitCommit as BackendGitCommit, Release as BackendRelease, TokenResponse,
};
use miru_agent::http::{self, request::Params, HTTPErr};

// external crates
use axum::http::StatusCode;
use axum::Json;
use axum::Router;
use serde::Serialize;
use tokio::net::TcpListener;
use tokio::time::Duration;

// ================================ MOCK CALL ======================================= //

#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    ProvisionDevice,
    ReprovisionDevice,
    IssueDeviceToken,
    UpdateDevice,
    GetDevice,
    ListDeployments,
    GetDeployment,
    UpdateDeployment,
    GetConfigInstanceContent,
    GetRelease,
    GetGitCommit,
}

// ================================ CAPTURED REQUEST ================================ //

#[derive(Clone, Debug, PartialEq)]
pub struct CapturedRequest {
    pub call: Call,
    pub method: reqwest::Method,
    pub path: String,
    pub url: String,
    pub query: Vec<(String, String)>,
    pub body: Option<String>,
    pub token: Option<String>,
}

// ================================ MOCK CLIENT ==================================== //

type ListDeploymentsFn = Mutex<Box<dyn Fn() -> Result<DeploymentList, HTTPErr> + Send + Sync>>;
type SingleDeploymentFn = Mutex<Box<dyn Fn() -> Result<BackendDeployment, HTTPErr> + Send + Sync>>;
type SingleReleaseFn = Mutex<Box<dyn Fn() -> Result<BackendRelease, HTTPErr> + Send + Sync>>;
type SingleGitCommitFn = Mutex<Box<dyn Fn() -> Result<BackendGitCommit, HTTPErr> + Send + Sync>>;
type GetCfgInstContentFn = Mutex<Box<dyn Fn(&str) -> Result<String, HTTPErr> + Send + Sync>>;
type UpdateDeviceFn = Mutex<Box<dyn Fn() -> Result<Device, HTTPErr> + Send + Sync>>;
type GetDeviceFn = Mutex<Box<dyn Fn() -> Result<Device, HTTPErr> + Send + Sync>>;

pub struct MockClient {
    pub provision_device_fn: Box<dyn Fn() -> Result<Device, HTTPErr> + Send + Sync>,
    pub reprovision_device_fn: Box<dyn Fn() -> Result<Device, HTTPErr> + Send + Sync>,
    pub issue_device_token_fn: Box<dyn Fn() -> Result<TokenResponse, HTTPErr> + Send + Sync>,
    pub update_device_fn: UpdateDeviceFn,
    pub get_device_fn: GetDeviceFn,
    pub list_deployments_fn: ListDeploymentsFn,
    pub get_deployment_fn: SingleDeploymentFn,
    pub update_deployment_fn: SingleDeploymentFn,
    pub get_release_fn: SingleReleaseFn,
    pub get_git_commit_fn: SingleGitCommitFn,
    pub get_cfg_inst_content_fn: GetCfgInstContentFn,
    pub requests: Arc<Mutex<Vec<CapturedRequest>>>,
}

impl Default for MockClient {
    fn default() -> Self {
        Self {
            provision_device_fn: Box::new(|| Ok(Device::default())),
            reprovision_device_fn: Box::new(|| Ok(Device::default())),
            issue_device_token_fn: Box::new(|| Ok(TokenResponse::default())),
            update_device_fn: Mutex::new(Box::new(|| Ok(Device::default()))),
            get_device_fn: Mutex::new(Box::new(|| Ok(Device::default()))),
            list_deployments_fn: Mutex::new(Box::new(|| Ok(DeploymentList::default()))),
            get_deployment_fn: Mutex::new(Box::new(|| Ok(BackendDeployment::default()))),
            update_deployment_fn: Mutex::new(Box::new(|| Ok(BackendDeployment::default()))),
            get_release_fn: Mutex::new(Box::new(|| Ok(BackendRelease::default()))),
            get_git_commit_fn: Mutex::new(Box::new(|| Ok(BackendGitCommit::default()))),
            get_cfg_inst_content_fn: Mutex::new(Box::new(|_id| Ok("{}".to_string()))),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl MockClient {
    pub fn set_update_device<F>(&self, f: F)
    where
        F: Fn() -> Result<Device, HTTPErr> + Send + Sync + 'static,
    {
        *self.update_device_fn.lock().unwrap() = Box::new(f);
    }

    pub fn set_get_device<F>(&self, f: F)
    where
        F: Fn() -> Result<Device, HTTPErr> + Send + Sync + 'static,
    {
        *self.get_device_fn.lock().unwrap() = Box::new(f);
    }

    pub fn set_list_all_deployments<F>(&self, f: F)
    where
        F: Fn() -> Result<Vec<BackendDeployment>, HTTPErr> + Send + Sync + 'static,
    {
        *self.list_deployments_fn.lock().unwrap() = Box::new(move || {
            let data = f()?;
            Ok(DeploymentList {
                total_count: Some(data.len() as i64),
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

    pub fn set_get_release<F>(&self, f: F)
    where
        F: Fn() -> Result<BackendRelease, HTTPErr> + Send + Sync + 'static,
    {
        *self.get_release_fn.lock().unwrap() = Box::new(f);
    }

    pub fn set_get_git_commit<F>(&self, f: F)
    where
        F: Fn() -> Result<BackendGitCommit, HTTPErr> + Send + Sync + 'static,
    {
        *self.get_git_commit_fn.lock().unwrap() = Box::new(f);
    }

    pub fn set_get_config_instance_content<F>(&self, f: F)
    where
        F: Fn(&str) -> Result<String, HTTPErr> + Send + Sync + 'static,
    {
        *self.get_cfg_inst_content_fn.lock().unwrap() = Box::new(f);
    }

    pub fn call_count(&self, target: Call) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.call == target)
            .count()
    }

    pub fn num_update_device_calls(&self) -> usize {
        self.call_count(Call::UpdateDevice)
    }

    pub fn num_get_device_calls(&self) -> usize {
        self.call_count(Call::GetDevice)
    }

    pub fn requests(&self) -> Vec<CapturedRequest> {
        self.requests.lock().unwrap().clone()
    }

    pub fn paths_for(&self, target: Call) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.call == target)
            .map(|r| r.path.clone())
            .collect()
    }

    fn match_route(method: &reqwest::Method, path: &str) -> Call {
        use reqwest::Method;
        match (method, path) {
            (m, p) if *m == Method::POST && p == "/devices/provision" => Call::ProvisionDevice,
            (m, p) if *m == Method::POST && p == "/devices/reprovision" => {
                Call::ReprovisionDevice
            }
            (m, p) if *m == Method::POST && p.ends_with("/devices/token") => Call::IssueDeviceToken,
            (m, p) if *m == Method::PATCH && p.starts_with("/devices/") => Call::UpdateDevice,
            (m, p) if *m == Method::GET && p == "/device" => Call::GetDevice,
            (m, p) if *m == Method::GET && p == "/deployments" => Call::ListDeployments,
            (m, p)
                if *m == Method::GET
                    && p.starts_with("/config_instances/")
                    && p.ends_with("/content") =>
            {
                Call::GetConfigInstanceContent
            }
            (m, p) if *m == Method::GET && p.starts_with("/deployments/") => Call::GetDeployment,
            (m, p) if *m == Method::PATCH && p.starts_with("/deployments/") => {
                Call::UpdateDeployment
            }
            (m, p) if *m == Method::GET && p.starts_with("/releases/") => Call::GetRelease,
            (m, p) if *m == Method::GET && p.starts_with("/git_commits/") => Call::GetGitCommit,
            _ => panic!("MockClient: unhandled route: {method} {path}"),
        }
    }

    fn handle_route(&self, call: &Call, path: &str) -> Result<String, HTTPErr> {
        match call {
            Call::ProvisionDevice => json(&(self.provision_device_fn)()?),
            Call::ReprovisionDevice => json(&(self.reprovision_device_fn)()?),
            Call::IssueDeviceToken => json(&(self.issue_device_token_fn)()?),
            Call::UpdateDevice => json(&(self.update_device_fn.lock().unwrap())()?),
            Call::GetDevice => json(&(self.get_device_fn.lock().unwrap())()?),
            Call::ListDeployments => {
                let list = (self.list_deployments_fn.lock().unwrap())()?;
                json(&list)
            }
            Call::GetDeployment => json(&(self.get_deployment_fn.lock().unwrap())()?),
            Call::UpdateDeployment => json(&(self.update_deployment_fn.lock().unwrap())()?),
            Call::GetRelease => json(&(self.get_release_fn.lock().unwrap())()?),
            Call::GetGitCommit => json(&(self.get_git_commit_fn.lock().unwrap())()?),
            Call::GetConfigInstanceContent => {
                // Extract ID from /config_instances/{id}/content
                let id = path
                    .strip_prefix("/config_instances/")
                    .and_then(|s| s.strip_suffix("/content"))
                    .unwrap_or("");
                (self.get_cfg_inst_content_fn.lock().unwrap())(id)
            }
        }
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
        let url_str = params.url;
        let path = url_str
            .strip_prefix(http::ClientI::base_url(self))
            .unwrap_or(url_str);
        let path = path.split('?').next().unwrap_or(path);
        let call = Self::match_route(&params.method, path);
        self.requests.lock().unwrap().push(CapturedRequest {
            call: call.clone(),
            method: params.method.clone(),
            path: path.to_string(),
            url: url_str.to_string(),
            query: params.query.clone(),
            body: params.body.clone(),
            token: params.token.map(|t| t.to_string()),
        });
        let text = self.handle_route(&call, path)?;
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
