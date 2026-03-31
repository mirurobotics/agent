// internal crates
use miru_agent::events::model::{
    DeploymentDeployedEvent, DeploymentRemovedEvent, Event, EventArgs, DEPLOYMENT_DEPLOYED,
    DEPLOYMENT_REMOVED,
};
use miru_agent::models::{Deployment, DplActivity, DplErrStatus, DplTarget};

// external crates
use chrono::{TimeZone, Utc};

fn fixed_time() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2025, 6, 15, 12, 0, 0).unwrap()
}

// ========================= EVENT TYPES ========================= //

mod event_types {
    use super::*;

    #[test]
    fn deployment_deployed_type_string() {
        assert_eq!(DEPLOYMENT_DEPLOYED, "deployment.deployed.beta1");
    }

    #[test]
    fn deployment_removed_type_string() {
        assert_eq!(DEPLOYMENT_REMOVED, "deployment.removed.beta1");
    }
}

// ========================= EVENT ========================= //

mod event {
    use super::*;

    #[test]
    fn serialization_roundtrip() {
        let event = Event {
            id: 7,
            event_type: "test.roundtrip".to_string(),
            occurred_at: Utc::now(),
            data: serde_json::json!({"deployment_id": "dpl-1"}),
        };

        let json = serde_json::to_string(&event).unwrap();
        let deserialized: Event = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, 7);
        assert_eq!(deserialized.event_type, "test.roundtrip");
        assert_eq!(deserialized.data["deployment_id"], "dpl-1");
    }

    #[test]
    fn type_field_serialized_as_type() {
        let event = Event {
            id: 1,
            event_type: "test.rename".to_string(),
            occurred_at: Utc::now(),
            data: serde_json::json!({}),
        };

        let json = serde_json::to_string(&event).unwrap();
        // The field is serialized as "type" not "event_type"
        assert!(json.contains("\"type\":\"test.rename\""));
    }
}

// ========================= DEPLOYMENT DEPLOYED ========================= //

mod deployment_deployed {
    use super::*;

    #[test]
    fn serializes_all_fields() {
        let t = fixed_time();
        let dpl = Deployment {
            id: "dpl-1".into(),
            activity_status: DplActivity::Deployed,
            error_status: DplErrStatus::None,
            target_status: DplTarget::Deployed,
            deployed_at: Some(t),
            ..Default::default()
        };

        let actual = EventArgs::deployed(&dpl).unwrap();
        assert_eq!(actual.event_type, DEPLOYMENT_DEPLOYED);
        assert_eq!(
            actual.data,
            serde_json::json!(DeploymentDeployedEvent {
                deployment_id: "dpl-1".into(),
                activity_status: "deployed".into(),
                target_status: "deployed".into(),
                deployed_at: Some(t.to_rfc3339()),
            })
        );
    }

    #[test]
    fn deployed_at_none_when_absent() {
        let dpl = Deployment {
            id: "dpl-1".into(),
            activity_status: DplActivity::Deployed,
            target_status: DplTarget::Deployed,
            deployed_at: None,
            ..Default::default()
        };

        let event = EventArgs::deployed(&dpl).unwrap();
        let actual: DeploymentDeployedEvent = serde_json::from_value(event.data).unwrap();
        assert_eq!(actual.deployed_at, None);
    }

    #[test]
    fn does_not_validate_activity_status() {
        let dpl = Deployment {
            id: "dpl-1".into(),
            activity_status: DplActivity::Queued,
            target_status: DplTarget::Deployed,
            ..Default::default()
        };

        let event = EventArgs::deployed(&dpl).unwrap();
        assert_eq!(event.event_type, DEPLOYMENT_DEPLOYED);
        let actual: DeploymentDeployedEvent = serde_json::from_value(event.data).unwrap();
        assert_eq!(actual.activity_status, "queued");
    }
}

// ========================= DEPLOYMENT REMOVED ========================= //

mod deployment_removed {
    use super::*;

    #[test]
    fn serializes_all_fields() {
        let t = fixed_time();
        let dpl = Deployment {
            id: "dpl-1".into(),
            activity_status: DplActivity::Archived,
            target_status: DplTarget::Archived,
            archived_at: Some(t),
            ..Default::default()
        };

        let actual = EventArgs::removed(&dpl).unwrap();
        assert_eq!(actual.event_type, DEPLOYMENT_REMOVED);
        assert_eq!(
            actual.data,
            serde_json::json!(DeploymentRemovedEvent {
                deployment_id: "dpl-1".into(),
                activity_status: "archived".into(),
                target_status: "archived".into(),
                archived_at: Some(t.to_rfc3339()),
            })
        );
    }

    #[test]
    fn archived_at_none_when_absent() {
        let dpl = Deployment {
            id: "dpl-1".into(),
            activity_status: DplActivity::Archived,
            target_status: DplTarget::Archived,
            archived_at: None,
            ..Default::default()
        };

        let event = EventArgs::removed(&dpl).unwrap();
        let actual: DeploymentRemovedEvent = serde_json::from_value(event.data).unwrap();
        assert_eq!(actual.archived_at, None);
    }

    #[test]
    fn does_not_validate_activity_status() {
        let dpl = Deployment {
            id: "dpl-1".into(),
            activity_status: DplActivity::Deployed,
            target_status: DplTarget::Archived,
            ..Default::default()
        };

        let event = EventArgs::removed(&dpl).unwrap();
        assert_eq!(event.event_type, DEPLOYMENT_REMOVED);
        let actual: DeploymentRemovedEvent = serde_json::from_value(event.data).unwrap();
        assert_eq!(actual.activity_status, "deployed");
    }
}
