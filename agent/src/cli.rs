#[derive(Debug, Default)]
pub struct Args {
    pub display_version: bool,
    pub install_args: Option<InstallArgs>,
}

impl Args {
    pub fn parse(inputs: &[String]) -> Self {
        let mut args = Self::default();
        for input in inputs.iter().skip(1) {
            match input.trim_start_matches('-') {
                "version" => args.display_version = true,
                "install" => args.install_args = Some(InstallArgs::parse(inputs)),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn to_inputs(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    mod args_parse {
        use super::*;

        #[test]
        fn parses_version_and_install_with_install_args() {
            let inputs = to_inputs(&[
                "miru-agent",
                "--version",
                "install",
                "--backend-host=https://backend.example.com",
                "--mqtt-broker-host=mqtt.example.com",
                "--device-name=robot-1",
            ]);

            let args = Args::parse(&inputs);

            assert!(args.display_version);
            assert!(args.install_args.is_some());

            let install_args = args.install_args.expect("install args should be present");
            assert_eq!(
                Some("https://backend.example.com"),
                install_args.backend_host.as_deref()
            );
            assert_eq!(
                Some("mqtt.example.com"),
                install_args.mqtt_broker_host.as_deref()
            );
            assert_eq!(Some("robot-1"), install_args.device_name.as_deref());
        }

        #[test]
        fn ignores_installer_options_without_install_flag() {
            let inputs = to_inputs(&[
                "miru-agent",
                "--backend-host=https://backend.example.com",
                "--mqtt-broker-host=mqtt.example.com",
            ]);

            let args = Args::parse(&inputs);

            assert!(!args.display_version);
            assert!(args.install_args.is_none());
        }

        #[test]
        fn empty_input_returns_defaults() {
            let inputs: Vec<String> = vec![];
            let args = Args::parse(&inputs);

            assert!(!args.display_version);
            assert!(args.install_args.is_none());
        }
    }

    mod install_args_parse {
        use super::*;

        #[test]
        fn parses_known_key_value_options() {
            let inputs = to_inputs(&[
                "miru-agent",
                "--backend-host=https://backend.example.com",
                "--mqtt-broker-host=mqtt.example.com",
                "--device-name=robot-1",
            ]);

            let args = InstallArgs::parse(&inputs);

            assert_eq!(
                Some("https://backend.example.com"),
                args.backend_host.as_deref()
            );
            assert_eq!(Some("mqtt.example.com"), args.mqtt_broker_host.as_deref());
            assert_eq!(Some("robot-1"), args.device_name.as_deref());
        }

        #[test]
        fn ignores_unknown_or_non_key_value_tokens() {
            let inputs = to_inputs(&[
                "miru-agent",
                "--unknown=value",
                "--backend-host=https://backend.example.com",
                "--device-name",
                "install",
            ]);

            let args = InstallArgs::parse(&inputs);

            assert_eq!(
                Some("https://backend.example.com"),
                args.backend_host.as_deref()
            );
            assert!(args.mqtt_broker_host.is_none());
            assert!(args.device_name.is_none());
        }

        #[test]
        fn last_duplicate_value_wins() {
            let inputs = to_inputs(&[
                "miru-agent",
                "--backend-host=https://first.example.com",
                "--backend-host=https://second.example.com",
            ]);

            let args = InstallArgs::parse(&inputs);

            assert_eq!(
                Some("https://second.example.com"),
                args.backend_host.as_deref()
            );
        }

        #[test]
        fn empty_values_are_treated_as_none() {
            let inputs = to_inputs(&["miru-agent", "--device-name="]);
            let args = InstallArgs::parse(&inputs);

            assert!(args.device_name.is_none());
        }
    }
}
