// standard crates
use std::sync::{Arc, Mutex};

// internal crates
use miru_agent::http::devices::DevicesExt;
use miru_agent::http::errors::HTTPErr;
use openapi_client::models::{
    ActivateDeviceRequest, Device, IssueDeviceTokenRequest,
    TokenResponse, UpdateDeviceFromAgentRequest,
};

// ================================ MOCK CLIENT ==================================== //

#[derive(Default)]
pub struct MockClient {
    pub devices_client: MockDevicesClient,
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

impl MockClient {
    // MockClient methods can be added here as needed
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


