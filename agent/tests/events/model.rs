use chrono::Utc;
use miru_agent::events::model::{self, DeploymentEventData, Envelope, SyncFailedData};
use miru_agent::events::Subject;
use miru_agent::models::{Deployment, DplTarget};

mod serde {
    use super::*;

    #[test]
    fn envelope_roundtrip() {
        let now = Utc::now();
        let env = Envelope {
            id: 42,
            event_type: "sync.completed".to_string(),
            schema_version: 1,
            occurred_at: now,
            device_id: "dev-1".to_string(),
            subject: Subject {
                resource_type: "device".to_string(),
                id: "dev-1".to_string(),
            },
            data: serde_json::json!({"last_synced_at": now}),
        };

        let json = serde_json::to_string(&env).unwrap();
        let parsed: Envelope = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.id, 42);
        assert_eq!(parsed.event_type, "sync.completed");
        assert_eq!(parsed.schema_version, 1);
        assert_eq!(parsed.device_id, "dev-1");
        assert_eq!(parsed.subject.resource_type, "device");
        assert_eq!(parsed.subject.id, "dev-1");
    }

    #[test]
    fn type_field_renamed_in_json() {
        let env = Envelope::sync_completed("dev-1", Utc::now());
        let json = serde_json::to_string(&env).unwrap();
        let raw: serde_json::Value = serde_json::from_str(&json).unwrap();

        // The struct field `event_type` should serialize as "type"
        assert!(raw.get("type").is_some(), "expected 'type' key in JSON");
        assert!(
            raw.get("event_type").is_none(),
            "should not have 'event_type' key"
        );
    }

    #[test]
    fn subject_type_field_renamed() {
        let env = Envelope::sync_completed("dev-1", Utc::now());
        let json = serde_json::to_string(&env).unwrap();
        let raw: serde_json::Value = serde_json::from_str(&json).unwrap();

        let subject = raw.get("subject").unwrap();
        assert!(
            subject.get("type").is_some(),
            "expected 'type' key in subject"
        );
        assert!(
            subject.get("resource_type").is_none(),
            "should not have 'resource_type' key in subject"
        );
    }
}

mod constructors {
    use super::*;

    fn make_deployment(id: &str) -> Deployment {
        Deployment {
            id: id.to_string(),
            target_status: DplTarget::Deployed,
            deployed_at: Some(Utc::now()),
            ..Default::default()
        }
    }

    #[test]
    fn deployment_deployed() {
        let dpl = make_deployment("dpl-1");
        let env = Envelope::deployment_deployed("dev-1", &dpl);

        assert_eq!(env.id, 0);
        assert_eq!(env.event_type, model::DEPLOYMENT_DEPLOYED);
        assert_eq!(env.schema_version, model::SCHEMA_VERSION);
        assert_eq!(env.device_id, "dev-1");
        assert_eq!(env.subject.resource_type, "deployment");
        assert_eq!(env.subject.id, "dpl-1");
    }

    #[test]
    fn deployment_removed() {
        let dpl = make_deployment("dpl-2");
        let env = Envelope::deployment_removed("dev-1", &dpl);

        assert_eq!(env.event_type, model::DEPLOYMENT_REMOVED);
        assert_eq!(env.subject.resource_type, "deployment");
        assert_eq!(env.subject.id, "dpl-2");
    }

    #[test]
    fn sync_completed() {
        let now = Utc::now();
        let env = Envelope::sync_completed("dev-1", now);

        assert_eq!(env.id, 0);
        assert_eq!(env.event_type, model::SYNC_COMPLETED);
        assert_eq!(env.schema_version, model::SCHEMA_VERSION);
        assert_eq!(env.device_id, "dev-1");
        assert_eq!(env.subject.resource_type, "device");
        assert_eq!(env.subject.id, "dev-1");
    }

    #[test]
    fn sync_failed() {
        let env = Envelope::sync_failed("dev-1", true, 3);

        assert_eq!(env.event_type, model::SYNC_FAILED);
        assert_eq!(env.subject.resource_type, "device");
        assert_eq!(env.subject.id, "dev-1");

        let data: SyncFailedData = serde_json::from_value(env.data).unwrap();
        assert!(data.is_network_conn_err);
        assert_eq!(data.err_streak, 3);
    }

    #[test]
    fn deployment_event_data_roundtrip() {
        let now = Utc::now();
        let data = DeploymentEventData {
            target_status: DplTarget::Deployed,
            deployed_at: Some(now),
            archived_at: None,
        };
        let json = serde_json::to_string(&data).unwrap();
        let parsed: DeploymentEventData = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.target_status, DplTarget::Deployed);
        assert_eq!(parsed.deployed_at, Some(now));
        assert_eq!(parsed.archived_at, None);
    }

    #[test]
    fn sync_failed_data_roundtrip() {
        let data = SyncFailedData {
            is_network_conn_err: false,
            err_streak: 7,
        };
        let json = serde_json::to_string(&data).unwrap();
        let parsed: SyncFailedData = serde_json::from_str(&json).unwrap();

        assert!(!parsed.is_network_conn_err);
        assert_eq!(parsed.err_streak, 7);
    }
}
