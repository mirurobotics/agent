// standard crates
use std::env;

// internal crates
use crate::errors::Trace;
use crate::http;
use crate::installer::{
    errors::{InstallErr, MissingEnvVarErr},
    install,
};
use crate::storage::{self, settings};
use backend_api::models as backend_client;

// external crates
#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

const API_KEY_ENV_VAR: &str = "MIRU_API_KEY";

const DEVICE_IS_ACTIVE_ERROR_CODE: &str = "device_is_active";

#[derive(Debug, thiserror::Error)]
#[error("device {device_id} is already activated and reactivation is not allowed")]
pub struct ReactivationNotAllowedErr {
    pub device_id: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ReactivationNotAllowedErr {}

#[derive(Debug, thiserror::Error)]
#[error("miru-agent provision must be run as root")]
pub struct NotRootErr {
    pub trace: Box<Trace>,
}

impl crate::errors::Error for NotRootErr {}

#[derive(Debug, thiserror::Error)]
#[error("systemctl error: {msg}")]
pub struct SystemdErr {
    pub msg: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for SystemdErr {}

#[derive(Debug, thiserror::Error)]
pub enum ProvisionErr {
    #[error(transparent)]
    MissingApiKeyErr(MissingEnvVarErr),
    #[error(transparent)]
    BackendErr(http::HTTPErr),
    #[error(transparent)]
    ReactivationNotAllowedErr(ReactivationNotAllowedErr),
    #[error(transparent)]
    InstallErr(InstallErr),
    #[error(transparent)]
    NotRootErr(NotRootErr),
    #[error(transparent)]
    SystemdErr(SystemdErr),
}

impl From<InstallErr> for ProvisionErr {
    fn from(e: InstallErr) -> Self {
        Self::InstallErr(e)
    }
}

crate::impl_error!(ProvisionErr {
    MissingApiKeyErr,
    BackendErr,
    ReactivationNotAllowedErr,
    InstallErr,
    NotRootErr,
    SystemdErr,
});

pub fn read_api_key_from_env() -> Result<String, ProvisionErr> {
    match env::var(API_KEY_ENV_VAR) {
        Ok(api_key) => Ok(api_key),
        Err(_) => {
            error!("The {API_KEY_ENV_VAR} environment variable is not set");
            Err(ProvisionErr::MissingApiKeyErr(MissingEnvVarErr {
                name: API_KEY_ENV_VAR.to_string(),
                trace: crate::trace!(),
            }))
        }
    }
}

pub async fn provision<PublicHTTPClientT, AgentHTTPClientT>(
    public_api_client: &PublicHTTPClientT,
    agent_client: &AgentHTTPClientT,
    layout: &storage::Layout,
    settings: &settings::Settings,
    api_key: &str,
    device_name: &str,
    allow_reactivation: bool,
) -> Result<backend_client::Device, ProvisionErr>
where
    PublicHTTPClientT: http::ClientI,
    AgentHTTPClientT: http::ClientI,
{
    // create or fetch the device by name on the public API
    let device = http::devices::create_or_fetch_device(
        public_api_client,
        http::devices::CreateOrFetchDeviceParams {
            name: device_name,
            api_key,
        },
    )
    .await
    .map_err(ProvisionErr::BackendErr)?;

    // request an activation token; map device_is_active into ReactivationNotAllowedErr
    let token_response = match http::devices::issue_activation_token(
        public_api_client,
        http::devices::IssueActivationTokenParams {
            id: &device.id,
            api_key,
            allow_reactivation,
        },
    )
    .await
    {
        Ok(t) => t,
        Err(http::HTTPErr::RequestFailed(rf)) => {
            let is_device_active = rf
                .error
                .as_ref()
                .map(|e| e.error.code == DEVICE_IS_ACTIVE_ERROR_CODE)
                .unwrap_or(false);
            if is_device_active {
                return Err(ProvisionErr::ReactivationNotAllowedErr(
                    ReactivationNotAllowedErr {
                        device_id: device.id.clone(),
                        trace: crate::trace!(),
                    },
                ));
            }
            return Err(ProvisionErr::BackendErr(http::HTTPErr::RequestFailed(rf)));
        }
        Err(e) => return Err(ProvisionErr::BackendErr(e)),
    };

    // hand off to the existing install flow
    install::install(
        agent_client,
        layout,
        settings,
        &token_response.token,
        Some(device_name.to_string()),
    )
    .await
    .map_err(ProvisionErr::InstallErr)
}
