pub mod exit_codes;

#[derive(Debug, Default)]
pub struct Args {
    pub display_version: bool,
    pub install_args: Option<InstallArgs>,
    pub provision_args: Option<ProvisionArgs>,
}

impl Args {
    pub fn parse(inputs: &[String]) -> Self {
        let mut args = Self::default();
        for input in inputs.iter().skip(1) {
            match input.trim_start_matches('-') {
                "version" => args.display_version = true,
                "install" => args.install_args = Some(InstallArgs::parse(inputs)),
                "provision" => args.provision_args = Some(ProvisionArgs::parse(inputs)),
                _ => {}
            }
        }
        args
    }
}

#[derive(Debug, Default)]
pub struct InstallArgs {
    pub backend_host: Option<String>,
    pub mqtt_broker_host: Option<String>,
    pub device_name: Option<String>,
}

impl InstallArgs {
    pub fn parse(inputs: &[String]) -> Self {
        let mut args = Self::default();
        for input in inputs.iter().skip(1) {
            if let Some((key, value)) = input.split_once('=') {
                let value = if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                };
                match key.trim_start_matches('-') {
                    "backend-host" => args.backend_host = value,
                    "mqtt-broker-host" => args.mqtt_broker_host = value,
                    "device-name" => args.device_name = value,
                    _ => {}
                }
            }
        }
        args
    }
}

#[derive(Debug, Default)]
pub struct ProvisionArgs {
    pub device_name: Option<String>,
    pub allow_reactivation: Option<bool>,
    pub backend_host: Option<String>,
    pub mqtt_broker_host: Option<String>,
}

impl ProvisionArgs {
    pub fn parse(inputs: &[String]) -> Self {
        let mut args = Self::default();
        for input in inputs.iter().skip(1) {
            if let Some((key, value)) = input.split_once('=') {
                let raw_value = if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                };
                match key.trim_start_matches('-') {
                    "device-name" => args.device_name = raw_value,
                    "backend-host" => args.backend_host = raw_value,
                    "mqtt-broker-host" => args.mqtt_broker_host = raw_value,
                    "allow-reactivation" => {
                        args.allow_reactivation = match value {
                            "true" => Some(true),
                            "false" => Some(false),
                            _ => None,
                        };
                    }
                    _ => {}
                }
            }
        }
        args
    }
}
