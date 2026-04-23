// standard crates
use std::env;
use std::process::Command;

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

const MIRU_SYSTEMD_UNIT: &str = "miru";

/// systemctl exit code for "Unit not loaded" — treated as a no-op
/// (fresh install where the unit file hasn't been written yet).
const SYSTEMCTL_UNIT_NOT_LOADED_EXIT_CODE: i32 = 5;

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

// ============================ ROOT PRIVILEGE CHECK ============================ //

/// Returns the effective UID of the calling process. Indirection through this
/// function lets tests inject a fake euid via
/// [`test_support::set_fake_euid`] (gated behind the `test` feature).
fn current_euid() -> u32 {
    #[cfg(feature = "test")]
    if let Some(fake) = test_support::fake_euid() {
        return fake;
    }
    // SAFETY: `geteuid` is a thread-safe POSIX call that takes no arguments
    // and cannot fail.
    unsafe { libc::geteuid() }
}

/// Returns `Ok(())` when the process is running as root (euid 0).
/// Returns `Err(ProvisionErr::NotRootErr)` otherwise.
///
/// Invoked from `run_provision()` in `main.rs` as the very first statement so
/// privilege failure short-circuits before any side effects (logging init,
/// env reads, arg parsing).
pub fn assert_root() -> Result<(), ProvisionErr> {
    if current_euid() != 0 {
        return Err(ProvisionErr::NotRootErr(NotRootErr {
            trace: crate::trace!(),
        }));
    }
    Ok(())
}

#[cfg(feature = "test")]
pub mod test_support {
    use std::cell::Cell;

    thread_local! {
        static FAKE_EUID: Cell<Option<u32>> = const { Cell::new(None) };
    }

    pub(super) fn fake_euid() -> Option<u32> {
        FAKE_EUID.with(|c| c.get())
    }

    /// Set the fake euid for the current test thread.
    pub fn set_fake_euid(euid: Option<u32>) {
        FAKE_EUID.with(|c| c.set(euid));
    }
}

// ============================== SYSTEMCTL TRAIT =============================== //

/// Abstraction over `systemctl` invocations so tests can mock them.
pub trait SystemctlI {
    fn stop(&self, unit: &str) -> Result<(), SystemdErr>;
    fn restart(&self, unit: &str) -> Result<(), SystemdErr>;
}

/// Real `systemctl` implementation that shells out to `systemctl` (resolved via PATH).
pub struct RealSystemctl;

impl SystemctlI for RealSystemctl {
    fn stop(&self, unit: &str) -> Result<(), SystemdErr> {
        run_systemctl(&["stop", unit])
    }

    fn restart(&self, unit: &str) -> Result<(), SystemdErr> {
        run_systemctl(&["restart", unit])
    }
}

fn run_systemctl(args: &[&str]) -> Result<(), SystemdErr> {
    let output = Command::new("systemctl")
        .args(args)
        .output()
        .map_err(|e| SystemdErr {
            msg: format!("failed to invoke systemctl {}: {e}", args.join(" ")),
            trace: crate::trace!(),
        })?;

    match output.status.code() {
        Some(0) => Ok(()),
        // Exit code 5: "Unit miru.service not loaded" — treat as no-op so
        // fresh installs (where the unit file hasn't been written yet) succeed.
        Some(code) if code == SYSTEMCTL_UNIT_NOT_LOADED_EXIT_CODE => Ok(()),
        Some(n) => Err(SystemdErr {
            msg: format!(
                "systemctl {} exited with code {}: {}",
                args.join(" "),
                n,
                String::from_utf8_lossy(&output.stderr)
            ),
            trace: crate::trace!(),
        }),
        None => Err(SystemdErr {
            msg: format!("systemctl {} killed by signal", args.join(" ")),
            trace: crate::trace!(),
        }),
    }
}

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

#[allow(clippy::too_many_arguments)]
pub async fn provision<PublicHTTPClientT, AgentHTTPClientT, SystemctlT>(
    public_api_client: &PublicHTTPClientT,
    agent_client: &AgentHTTPClientT,
    systemctl: &SystemctlT,
    layout: &storage::Layout,
    settings: &settings::Settings,
    api_key: &str,
    device_name: &str,
    allow_reactivation: bool,
) -> Result<backend_client::Device, ProvisionErr>
where
    PublicHTTPClientT: http::ClientI,
    AgentHTTPClientT: http::ClientI,
    SystemctlT: SystemctlI,
{
    // stop the miru systemd unit (if loaded) before touching /srv/miru
    systemctl
        .stop(MIRU_SYSTEMD_UNIT)
        .map_err(ProvisionErr::SystemdErr)?;

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
    let device = install::install(
        agent_client,
        layout,
        settings,
        &token_response.token,
        Some(device_name.to_string()),
    )
    .await
    .map_err(ProvisionErr::InstallErr)?;

    // restart the miru systemd unit so the freshly-installed credentials are
    // picked up. Failure here is loud: install succeeded but the operator must
    // intervene before the agent is healthy again.
    systemctl
        .restart(MIRU_SYSTEMD_UNIT)
        .map_err(ProvisionErr::SystemdErr)?;

    Ok(device)
}
