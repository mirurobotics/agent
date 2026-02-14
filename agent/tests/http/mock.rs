// standard crates
use std::fmt;
use std::sync::{Arc, Mutex};

// internal crates
use miru_agent::http::deployments::DeploymentsExt;
use miru_agent::http::devices::DevicesExt;
use miru_agent::http::errors::HTTPErr;
use miru_agent::http::pagination::Pagination;
use openapi_client::models::{
    ActivateDeviceRequest, Deployment as BackendDeployment, DeploymentActivityStatus,
    DeploymentList, Device, IssueDeviceTokenRequest, TokenResponse, UpdateDeploymentRequest,
    UpdateDeviceFromAgentRequest,
};

// ================================ MOCK CLIENT ==================================== //

#[derive(Default)]
pub struct MockClient {
    pub devices_client: MockDevicesClient,
    pub deployments_client: MockDeploymentsClient,
}

impl DevicesExt for MockClient {
    async fn activate_device(
        &self,
        device_id: &str,
        payload: &ActivateDeviceRequest,
        token: &str,
    ) -> Result<Device, HTTPErr> {
        self.devices_client
            .activate_device(device_id, payload, token)
            .await
    }

    async fn issue_device_token(
        &self,
        device_id: &str,
        payload: &IssueDeviceTokenRequest,
    ) -> Result<TokenResponse, HTTPErr> {
        self.devices_client
            .issue_device_token(device_id, payload)
            .await
    }

    async fn update_device(
        &self,
        device_id: &str,
        payload: &UpdateDeviceFromAgentRequest,
        token: &str,
    ) -> Result<Device, HTTPErr> {
        self.devices_client
            .update_device(device_id, payload, token)
            .await
    }
}

impl DeploymentsExt for MockClient {
    async fn list_deployments<I>(
        &self,
        activity_status_filter: &[DeploymentActivityStatus],
        expansions: I,
        pagination: &Pagination,
        token: &str,
    ) -> Result<DeploymentList, HTTPErr>
    where
        I: IntoIterator + Send,
        I::Item: fmt::Display,
    {
        self.deployments_client
            .list_deployments(activity_status_filter, expansions, pagination, token)
            .await
    }

    async fn list_all_deployments<I>(
        &self,
        activity_status_filter: &[DeploymentActivityStatus],
        expansions: I,
        token: &str,
    ) -> Result<Vec<BackendDeployment>, HTTPErr>
    where
        I: IntoIterator + Send + Clone,
        I::Item: fmt::Display,
    {
        self.deployments_client
            .list_all_deployments(activity_status_filter, expansions, token)
            .await
    }

    async fn get_deployment<I>(
        &self,
        deployment_id: &str,
        expansions: I,
        token: &str,
    ) -> Result<BackendDeployment, HTTPErr>
    where
        I: IntoIterator + Send,
        I::Item: fmt::Display,
    {
        self.deployments_client
            .get_deployment(deployment_id, expansions, token)
            .await
    }

    async fn update_deployment<I>(
        &self,
        deployment_id: &str,
        updates: &UpdateDeploymentRequest,
        expansions: I,
        token: &str,
    ) -> Result<BackendDeployment, HTTPErr>
    where
        I: IntoIterator + Send,
        I::Item: fmt::Display,
    {
        self.deployments_client
            .update_deployment(deployment_id, updates, expansions, token)
            .await
    }
}

// ================================== DEVICES ====================================== //
#[derive(Clone, Debug, PartialEq)]
pub enum DevicesCall {
    ActivateDevice,
    IssueDeviceToken,
    UpdateDevice,
}

pub struct MockDevicesClient {
    pub activate_device_fn: Box<dyn Fn() -> Result<Device, HTTPErr> + Send + Sync>,
    pub issue_device_token_fn: Box<dyn Fn() -> Result<TokenResponse, HTTPErr> + Send + Sync>,
    pub update_device_fn: Box<dyn Fn() -> Result<Device, HTTPErr> + Send + Sync>,
    pub calls: Arc<Mutex<Vec<DevicesCall>>>,
}

impl Default for MockDevicesClient {
    fn default() -> Self {
        Self {
            activate_device_fn: Box::new(|| Ok(Device::default())),
            issue_device_token_fn: Box::new(|| Ok(TokenResponse::default())),
            update_device_fn: Box::new(|| Ok(Device::default())),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl DevicesExt for MockDevicesClient {
    async fn activate_device(
        &self,
        _: &str,
        _: &ActivateDeviceRequest,
        _: &str,
    ) -> Result<Device, HTTPErr> {
        (self.activate_device_fn)()
    }

    async fn issue_device_token(
        &self,
        _: &str,
        _: &IssueDeviceTokenRequest,
    ) -> Result<TokenResponse, HTTPErr> {
        (self.issue_device_token_fn)()
    }

    async fn update_device(
        &self,
        _: &str,
        _: &UpdateDeviceFromAgentRequest,
        _: &str,
    ) -> Result<Device, HTTPErr> {
        self.calls.lock().unwrap().push(DevicesCall::UpdateDevice);
        (self.update_device_fn)()
    }
}

impl MockDevicesClient {
    pub fn num_update_device_calls(&self) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| **call == DevicesCall::UpdateDevice)
            .count()
    }
}

// ================================ DEPLOYMENTS ==================================== //

type ListAllDeploymentsFn =
    Mutex<Box<dyn Fn() -> Result<Vec<BackendDeployment>, HTTPErr> + Send + Sync>>;
type UpdateDeploymentFn = Mutex<Box<dyn Fn() -> Result<BackendDeployment, HTTPErr> + Send + Sync>>;

#[derive(Clone, Debug, PartialEq)]
pub enum DeploymentsCall {
    ListAllDeployments,
    UpdateDeployment,
}

pub struct MockDeploymentsClient {
    pub list_all_deployments_fn: ListAllDeploymentsFn,
    pub update_deployment_fn: UpdateDeploymentFn,
    pub calls: Arc<Mutex<Vec<DeploymentsCall>>>,
}

impl Default for MockDeploymentsClient {
    fn default() -> Self {
        Self {
            list_all_deployments_fn: Mutex::new(Box::new(|| Ok(vec![]))),
            update_deployment_fn: Mutex::new(Box::new(|| Ok(BackendDeployment::default()))),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl MockDeploymentsClient {
    pub fn set_list_all_deployments<F>(&self, f: F)
    where
        F: Fn() -> Result<Vec<BackendDeployment>, HTTPErr> + Send + Sync + 'static,
    {
        *self.list_all_deployments_fn.lock().unwrap() = Box::new(f);
    }

    pub fn set_update_deployment<F>(&self, f: F)
    where
        F: Fn() -> Result<BackendDeployment, HTTPErr> + Send + Sync + 'static,
    {
        *self.update_deployment_fn.lock().unwrap() = Box::new(f);
    }

    pub fn num_update_deployment_calls(&self) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| **call == DeploymentsCall::UpdateDeployment)
            .count()
    }
}

impl DeploymentsExt for MockDeploymentsClient {
    async fn list_deployments<I>(
        &self,
        _: &[DeploymentActivityStatus],
        _: I,
        _: &Pagination,
        _: &str,
    ) -> Result<DeploymentList, HTTPErr>
    where
        I: IntoIterator + Send,
        I::Item: fmt::Display,
    {
        // For mock purposes, wrap list_all into a DeploymentList
        let data = (self.list_all_deployments_fn.lock().unwrap())()?;
        Ok(DeploymentList::new(
            openapi_client::models::deployment_list::Object::List,
            data.len() as i64,
            100,
            0,
            false,
            data,
        ))
    }

    async fn list_all_deployments<I>(
        &self,
        _: &[DeploymentActivityStatus],
        _: I,
        _: &str,
    ) -> Result<Vec<BackendDeployment>, HTTPErr>
    where
        I: IntoIterator + Send + Clone,
        I::Item: fmt::Display,
    {
        self.calls
            .lock()
            .unwrap()
            .push(DeploymentsCall::ListAllDeployments);
        (self.list_all_deployments_fn.lock().unwrap())()
    }

    async fn get_deployment<I>(&self, _: &str, _: I, _: &str) -> Result<BackendDeployment, HTTPErr>
    where
        I: IntoIterator + Send,
        I::Item: fmt::Display,
    {
        (self.update_deployment_fn.lock().unwrap())()
    }

    async fn update_deployment<I>(
        &self,
        _: &str,
        _: &UpdateDeploymentRequest,
        _: I,
        _: &str,
    ) -> Result<BackendDeployment, HTTPErr>
    where
        I: IntoIterator + Send,
        I::Item: fmt::Display,
    {
        self.calls
            .lock()
            .unwrap()
            .push(DeploymentsCall::UpdateDeployment);
        (self.update_deployment_fn.lock().unwrap())()
    }
}
