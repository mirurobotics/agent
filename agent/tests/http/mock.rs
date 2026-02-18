// standard crates
use std::sync::{Arc, Mutex};

// internal crates
use miru_agent::http;
use miru_agent::http::errors::HTTPErr;
use miru_agent::http::request::Params;
use openapi_client::models::{
    deployment_list, Deployment as BackendDeployment, DeploymentList, Device, TokenResponse,
};

// external crates
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

// ================================ MOCK CLIENT ==================================== //

type ListAllDeploymentsFn =
    Mutex<Box<dyn Fn() -> Result<Vec<BackendDeployment>, HTTPErr> + Send + Sync>>;
type SingleDeploymentFn = Mutex<Box<dyn Fn() -> Result<BackendDeployment, HTTPErr> + Send + Sync>>;

pub struct MockClient {
    pub activate_device_fn: Box<dyn Fn() -> Result<Device, HTTPErr> + Send + Sync>,
    pub issue_device_token_fn: Box<dyn Fn() -> Result<TokenResponse, HTTPErr> + Send + Sync>,
    pub update_device_fn: Box<dyn Fn() -> Result<Device, HTTPErr> + Send + Sync>,
    pub list_all_deployments_fn: ListAllDeploymentsFn,
    pub get_deployment_fn: SingleDeploymentFn,
    pub update_deployment_fn: SingleDeploymentFn,
    pub calls: Arc<Mutex<Vec<MockCall>>>,
}

impl Default for MockClient {
    fn default() -> Self {
        Self {
            activate_device_fn: Box::new(|| Ok(Device::default())),
            issue_device_token_fn: Box::new(|| Ok(TokenResponse::default())),
            update_device_fn: Box::new(|| Ok(Device::default())),
            list_all_deployments_fn: Mutex::new(Box::new(|| Ok(vec![]))),
            get_deployment_fn: Mutex::new(Box::new(|| Ok(BackendDeployment::default()))),
            update_deployment_fn: Mutex::new(Box::new(|| Ok(BackendDeployment::default()))),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl MockClient {
    pub fn set_list_all_deployments<F>(&self, f: F)
    where
        F: Fn() -> Result<Vec<BackendDeployment>, HTTPErr> + Send + Sync + 'static,
    {
        *self.list_all_deployments_fn.lock().unwrap() = Box::new(f);
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

    pub fn num_update_device_calls(&self) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| **call == MockCall::UpdateDevice)
            .count()
    }

    fn dispatch(&self, method: &reqwest::Method, url: &str) -> Result<String, HTTPErr> {
        let path = url
            .strip_prefix(http::ClientI::base_url(self))
            .unwrap_or(url);
        let path = path.split('?').next().unwrap_or(path);

        if *method == reqwest::Method::POST && path.ends_with("/activate") {
            self.calls.lock().unwrap().push(MockCall::ActivateDevice);
            let result = (self.activate_device_fn)()?;
            Ok(serde_json::to_string(&result).unwrap())
        } else if *method == reqwest::Method::POST && path.ends_with("/issue_token") {
            self.calls.lock().unwrap().push(MockCall::IssueDeviceToken);
            let result = (self.issue_device_token_fn)()?;
            Ok(serde_json::to_string(&result).unwrap())
        } else if *method == reqwest::Method::PATCH && path.starts_with("/devices/") {
            self.calls.lock().unwrap().push(MockCall::UpdateDevice);
            let result = (self.update_device_fn)()?;
            Ok(serde_json::to_string(&result).unwrap())
        } else if *method == reqwest::Method::GET && path == "/deployments" {
            self.calls.lock().unwrap().push(MockCall::ListDeployments);
            let data = (self.list_all_deployments_fn.lock().unwrap())()?;
            let list = DeploymentList::new(
                deployment_list::Object::List,
                data.len() as i64,
                100,
                0,
                false,
                data,
            );
            Ok(serde_json::to_string(&list).unwrap())
        } else if *method == reqwest::Method::GET && path.starts_with("/deployments/") {
            self.calls.lock().unwrap().push(MockCall::GetDeployment);
            let result = (self.get_deployment_fn.lock().unwrap())()?;
            Ok(serde_json::to_string(&result).unwrap())
        } else if *method == reqwest::Method::PATCH && path.starts_with("/deployments/") {
            self.calls.lock().unwrap().push(MockCall::UpdateDeployment);
            let result = (self.update_deployment_fn.lock().unwrap())()?;
            Ok(serde_json::to_string(&result).unwrap())
        } else {
            panic!("MockClient: unhandled route: {} {}", method, url)
        }
    }
}

impl http::ClientI for MockClient {
    fn base_url(&self) -> &str {
        "http://mock"
    }

    fn default_timeout(&self) -> Duration {
        Duration::from_secs(10)
    }

    async fn execute(&self, params: Params<'_>) -> Result<String, HTTPErr> {
        self.dispatch(&params.method, params.url)
    }

    async fn execute_cached(&self, params: Params<'_>) -> Result<String, HTTPErr> {
        self.execute(params).await
    }
}
