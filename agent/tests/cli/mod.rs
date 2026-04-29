// internal crates
use miru_agent::cli::{Args, ProvisionArgs, ReprovisionArgs};

fn to_inputs(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

mod args_parse {
    use super::*;

    #[test]
    fn parses_version_and_provision_with_provision_args() {
        let inputs = to_inputs(&[
            "miru-agent",
            "--version",
            "provision",
            "--backend-host=https://backend.example.com",
            "--mqtt-broker-host=mqtt.example.com",
            "--device-name=robot-1",
        ]);

        let args = Args::parse(&inputs);

        assert!(args.display_version);
        assert!(args.provision_args.is_some());

        let provision_args = args
            .provision_args
            .expect("provision args should be present");
        assert_eq!(
            Some("https://backend.example.com"),
            provision_args.backend_host.as_deref()
        );
        assert_eq!(
            Some("mqtt.example.com"),
            provision_args.mqtt_broker_host.as_deref()
        );
        assert_eq!(Some("robot-1"), provision_args.device_name.as_deref());
    }

    #[test]
    fn ignores_provision_options_without_provision_flag() {
        let inputs = to_inputs(&[
            "miru-agent",
            "--backend-host=https://backend.example.com",
            "--mqtt-broker-host=mqtt.example.com",
        ]);

        let args = Args::parse(&inputs);

        assert!(!args.display_version);
        assert!(args.provision_args.is_none());
    }

    #[test]
    fn empty_input_returns_defaults() {
        let inputs: Vec<String> = vec![];
        let args = Args::parse(&inputs);

        assert!(!args.display_version);
        assert!(args.provision_args.is_none());
        assert!(args.reprovision_args.is_none());
    }

    #[test]
    fn parses_reprovision_subcommand_with_reprovision_args() {
        let inputs = to_inputs(&[
            "miru-agent",
            "reprovision",
            "--backend-host=https://backend.example.com",
            "--mqtt-broker-host=mqtt.example.com",
        ]);

        let args = Args::parse(&inputs);

        assert!(args.reprovision_args.is_some());
        assert!(args.provision_args.is_none());

        let reprovision_args = args
            .reprovision_args
            .expect("reprovision args should be present");
        assert_eq!(
            Some("https://backend.example.com"),
            reprovision_args.backend_host.as_deref()
        );
        assert_eq!(
            Some("mqtt.example.com"),
            reprovision_args.mqtt_broker_host.as_deref()
        );
    }

    #[test]
    fn ignores_reprovision_options_without_reprovision_flag() {
        let inputs = to_inputs(&["miru-agent", "--backend-host=https://backend.example.com"]);

        let args = Args::parse(&inputs);

        assert!(args.reprovision_args.is_none());
    }

    #[test]
    fn recognizes_provision_and_reprovision_independently() {
        let provision_only = to_inputs(&["miru-agent", "provision"]);
        let provision_args = Args::parse(&provision_only);
        assert!(provision_args.provision_args.is_some());
        assert!(provision_args.reprovision_args.is_none());

        let reprovision_only = to_inputs(&["miru-agent", "reprovision"]);
        let reprovision_args = Args::parse(&reprovision_only);
        assert!(reprovision_args.provision_args.is_none());
        assert!(reprovision_args.reprovision_args.is_some());
    }
}

mod provision_args_parse {
    use super::*;

    #[test]
    fn parses_known_key_value_options() {
        let inputs = to_inputs(&[
            "miru-agent",
            "--backend-host=https://backend.example.com",
            "--mqtt-broker-host=mqtt.example.com",
            "--device-name=robot-1",
        ]);

        let args = ProvisionArgs::parse(&inputs);

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
            "provision",
        ]);

        let args = ProvisionArgs::parse(&inputs);

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

        let args = ProvisionArgs::parse(&inputs);

        assert_eq!(
            Some("https://second.example.com"),
            args.backend_host.as_deref()
        );
    }

    #[test]
    fn empty_values_are_treated_as_none() {
        let inputs = to_inputs(&["miru-agent", "--device-name="]);
        let args = ProvisionArgs::parse(&inputs);

        assert!(args.device_name.is_none());
    }
}

mod reprovision_args_parse {
    use super::*;

    #[test]
    fn parses_known_key_value_options() {
        let inputs = to_inputs(&[
            "miru-agent",
            "--backend-host=https://backend.example.com",
            "--mqtt-broker-host=mqtt.example.com",
            "--device-name=robot-1",
        ]);

        let args = ReprovisionArgs::parse(&inputs);

        assert_eq!(
            Some("https://backend.example.com"),
            args.backend_host.as_deref()
        );
        assert_eq!(Some("mqtt.example.com"), args.mqtt_broker_host.as_deref());
        // ReprovisionArgs has no device_name field by design — the value is silently ignored.
    }

    #[test]
    fn ignores_unknown_or_non_key_value_tokens() {
        let inputs = to_inputs(&[
            "miru-agent",
            "--unknown=value",
            "--backend-host=https://backend.example.com",
            "--mqtt-broker-host",
            "reprovision",
        ]);

        let args = ReprovisionArgs::parse(&inputs);

        assert_eq!(
            Some("https://backend.example.com"),
            args.backend_host.as_deref()
        );
        assert!(args.mqtt_broker_host.is_none());
    }

    #[test]
    fn last_duplicate_value_wins() {
        let inputs = to_inputs(&[
            "miru-agent",
            "--backend-host=https://first.example.com",
            "--backend-host=https://second.example.com",
        ]);

        let args = ReprovisionArgs::parse(&inputs);

        assert_eq!(
            Some("https://second.example.com"),
            args.backend_host.as_deref()
        );
    }

    #[test]
    fn empty_values_are_treated_as_none() {
        let inputs = to_inputs(&["miru-agent", "--backend-host="]);
        let args = ReprovisionArgs::parse(&inputs);

        assert!(args.backend_host.is_none());
    }
}
