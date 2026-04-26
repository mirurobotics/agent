#[derive(Debug, Default)]
pub struct Args {
    pub display_version: bool,
    pub provision_args: Option<ProvisionArgs>,
}

impl Args {
    pub fn parse(inputs: &[String]) -> Self {
        let mut args = Self::default();
        for input in inputs.iter().skip(1) {
            match input.trim_start_matches('-') {
                "version" => args.display_version = true,
                "provision" => args.provision_args = Some(ProvisionArgs::parse(inputs)),
                _ => {}
            }
        }
        args
    }
}

#[derive(Debug, Default)]
pub struct ProvisionArgs {
    pub backend_host: Option<String>,
    pub mqtt_broker_host: Option<String>,
    pub device_name: Option<String>,
}

impl ProvisionArgs {
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
