// internal crates
use miru_agent::mqtt::topics;

mod format {
    use super::*;

    #[test]
    fn device_sync_format() {
        // device_sync uses legacy format without version prefix
        assert_eq!(topics::device_sync("dev-001"), "cmd/devices/dev-001/sync");
    }

    #[test]
    fn device_ping_format() {
        assert_eq!(
            topics::device_ping("dev-001"),
            "v1/cmd/devices/dev-001/ping"
        );
    }

    #[test]
    fn device_pong_format() {
        assert_eq!(
            topics::device_pong("dev-001"),
            "v1/resp/devices/dev-001/pong"
        );
    }
}

mod parse_subscription {
    use super::*;

    #[test]
    fn sync() {
        let topic = topics::device_sync("123");
        assert_eq!(
            topics::parse_subscription("123", &topic),
            topics::SubscriptionTopics::Sync
        );
    }

    #[test]
    fn ping() {
        let topic = topics::device_ping("123");
        assert_eq!(
            topics::parse_subscription("123", &topic),
            topics::SubscriptionTopics::Ping
        );
    }

    #[test]
    fn pong_is_unknown() {
        // pong is a response topic, not a subscription topic
        let topic = topics::device_pong("123");
        assert_eq!(
            topics::parse_subscription("123", &topic),
            topics::SubscriptionTopics::Unknown
        );
    }

    #[test]
    fn wrong_device_id() {
        // topic for device "abc" parsed with device "xyz" should be Unknown
        let topic = topics::device_sync("abc");
        assert_eq!(
            topics::parse_subscription("xyz", &topic),
            topics::SubscriptionTopics::Unknown
        );

        let topic = topics::device_ping("abc");
        assert_eq!(
            topics::parse_subscription("xyz", &topic),
            topics::SubscriptionTopics::Unknown
        );
    }

    #[test]
    fn unknown_topics() {
        let unknown_topics = vec![
            "v1/cmd/devices/123/unknown",
            "v2/cmd/devices/123/ping",
            "arglechargle",
            "",
        ];
        for topic in unknown_topics {
            assert_eq!(
                topics::parse_subscription("123", topic),
                topics::SubscriptionTopics::Unknown,
                "expected Unknown for topic: {topic}"
            );
        }
    }
}
