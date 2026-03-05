// internal crates
use miru_agent::cli::{Args, InstallArgs};

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
