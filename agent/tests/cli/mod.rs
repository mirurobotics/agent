// internal crates
use miru_agent::cli::{Args, InstallArgs, ProvisionArgs};

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

mod provision_args_parse {
    use super::*;

    #[test]
    fn parses_device_name_only() {
        let inputs = to_inputs(&["miru-agent", "provision", "--device-name=foo"]);

        let args = Args::parse(&inputs);

        assert!(args.provision_args.is_some());
        let provision_args = args
            .provision_args
            .expect("provision args should be present");
        assert_eq!(Some("foo"), provision_args.device_name.as_deref());
        assert_eq!(None, provision_args.allow_reactivation);
        assert!(provision_args.backend_host.is_none());
        assert!(provision_args.mqtt_broker_host.is_none());
    }

    #[test]
    fn parses_allow_reactivation_false() {
        let inputs = to_inputs(&[
            "miru-agent",
            "provision",
            "--device-name=foo",
            "--allow-reactivation=false",
        ]);

        let args = Args::parse(&inputs);
        let provision_args = args
            .provision_args
            .expect("provision args should be present");

        assert_eq!(Some("foo"), provision_args.device_name.as_deref());
        assert_eq!(Some(false), provision_args.allow_reactivation);
    }

    #[test]
    fn parses_all_fields() {
        let inputs = to_inputs(&[
            "miru-agent",
            "provision",
            "--device-name=foo",
            "--allow-reactivation=true",
            "--backend-host=https://x",
            "--mqtt-broker-host=mqtt://y",
        ]);

        let args = Args::parse(&inputs);
        let provision_args = args
            .provision_args
            .expect("provision args should be present");

        assert_eq!(Some("foo"), provision_args.device_name.as_deref());
        assert_eq!(Some(true), provision_args.allow_reactivation);
        assert_eq!(Some("https://x"), provision_args.backend_host.as_deref());
        assert_eq!(Some("mqtt://y"), provision_args.mqtt_broker_host.as_deref());
    }

    #[test]
    fn no_args_yields_all_none_fields() {
        let inputs = to_inputs(&["miru-agent", "provision"]);

        let args = Args::parse(&inputs);
        let provision_args = args
            .provision_args
            .expect("provision args should be present");

        assert!(provision_args.device_name.is_none());
        assert_eq!(None, provision_args.allow_reactivation);
        assert!(provision_args.backend_host.is_none());
        assert!(provision_args.mqtt_broker_host.is_none());
    }

    #[test]
    fn parses_directly_via_provision_args_parse() {
        let inputs = to_inputs(&[
            "miru-agent",
            "provision",
            "--device-name=robot-7",
            "--allow-reactivation=true",
        ]);

        let args = ProvisionArgs::parse(&inputs);

        assert_eq!(Some("robot-7"), args.device_name.as_deref());
        assert_eq!(Some(true), args.allow_reactivation);
    }

    #[test]
    fn empty_value_after_equals_is_none() {
        let inputs = to_inputs(&["miru-agent", "provision", "--device-name="]);

        let args = ProvisionArgs::parse(&inputs);

        assert!(args.device_name.is_none());
    }

    #[test]
    fn allow_reactivation_with_invalid_value_is_none() {
        let inputs = to_inputs(&[
            "miru-agent",
            "provision",
            "--device-name=robot-9",
            "--allow-reactivation=maybe",
        ]);

        let args = ProvisionArgs::parse(&inputs);

        assert_eq!(Some("robot-9"), args.device_name.as_deref());
        assert!(args.allow_reactivation.is_none());
    }

    #[test]
    fn unrecognized_flag_is_ignored() {
        let inputs = to_inputs(&[
            "miru-agent",
            "provision",
            "--device-name=robot-10",
            "--something-else=foo",
        ]);

        let args = ProvisionArgs::parse(&inputs);

        assert_eq!(Some("robot-10"), args.device_name.as_deref());
    }
}
