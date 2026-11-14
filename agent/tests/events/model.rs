// internal crates
use miru_agent::events::errors::EventsErr;
use miru_agent::events::model::{
    DeploymentDeployedEvent, DeploymentRemovedEvent, Event, EventArgs, EventPayload,
    DEPLOYMENT_DEPLOYED, DEPLOYMENT_REMOVED,
};
use miru_agent::models::{Deployment, DplActivity, DplErrStatus, DplStatus, DplTarget};

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
        assert_eq!(DEPLOYMENT_DEPLOYED, "deployment.deployed");
    }

    #[test]
    fn deployment_removed_type_string() {
        assert_eq!(DEPLOYMENT_REMOVED, "deployment.removed");
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
            release_id: "rls-1".into(),
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
            serde_json::json!({
                "deployment_id": "dpl-1",
                "release_id": "rls-1",
                "status": "deployed",
                "activity_status": "deployed",
                "error_status": "none",
                "target_status": "deployed",
                "deployed_at": t.to_rfc3339(),
            })
        );
    }

    #[test]
    fn omits_deployed_at_when_absent() {
        let dpl = Deployment {
            id: "dpl-1".into(),
            release_id: "rls-1".into(),
            activity_status: DplActivity::Deployed,
            target_status: DplTarget::Deployed,
            deployed_at: None,
            ..Default::default()
        };

        let event = EventArgs::deployed(&dpl).unwrap();
        assert_eq!(
            event.data,
            serde_json::json!({
                "deployment_id": "dpl-1",
                "release_id": "rls-1",
                "status": "deployed",
                "activity_status": "deployed",
                "error_status": "none",
                "target_status": "deployed",
            })
        );
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
        assert_eq!(actual.activity_status, DplActivity::Queued);
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
            release_id: "rls-1".into(),
            activity_status: DplActivity::Archived,
            target_status: DplTarget::Archived,
            archived_at: Some(t),
            ..Default::default()
        };

        let actual = EventArgs::removed(&dpl).unwrap();
        assert_eq!(actual.event_type, DEPLOYMENT_REMOVED);
        assert_eq!(
            actual.data,
            serde_json::json!({
                "deployment_id": "dpl-1",
                "release_id": "rls-1",
                "status": "archived",
                "activity_status": "archived",
                "error_status": "none",
                "target_status": "archived",
                "archived_at": t.to_rfc3339(),
            })
        );
    }

    #[test]
    fn omits_archived_at_when_absent() {
        let dpl = Deployment {
            id: "dpl-1".into(),
            release_id: "rls-1".into(),
            activity_status: DplActivity::Archived,
            target_status: DplTarget::Archived,
            archived_at: None,
            ..Default::default()
        };

        let event = EventArgs::removed(&dpl).unwrap();
        assert_eq!(
            event.data,
            serde_json::json!({
                "deployment_id": "dpl-1",
                "release_id": "rls-1",
                "status": "archived",
                "activity_status": "archived",
                "error_status": "none",
                "target_status": "archived",
            })
        );
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
        assert_eq!(actual.activity_status, DplActivity::Deployed);
    }
}

// ========================= PAYLOAD ========================= //

mod payload {
    use super::*;

    fn stored(event_type: &str, data: serde_json::Value) -> Event {
        Event {
            id: 1,
            event_type: event_type.to_string(),
            occurred_at: fixed_time(),
            data,
        }
    }

    #[test]
    fn parses_deployed_event() {
        let t = fixed_time().to_rfc3339();
        let event = stored(
            DEPLOYMENT_DEPLOYED,
            serde_json::json!({
                "deployment_id": "dpl-1",
                "release_id": "rls-1",
                "status": "failed",
                "activity_status": "queued",
                "error_status": "failed",
                "target_status": "deployed",
                "deployed_at": t,
            }),
        );

        let expected = EventPayload::Deployed(DeploymentDeployedEvent {
            deployment_id: "dpl-1".into(),
            release_id: "rls-1".into(),
            status: DplStatus::Failed,
            activity_status: DplActivity::Queued,
            error_status: DplErrStatus::Failed,
            target_status: DplTarget::Deployed,
            deployed_at: Some(t),
        });
        assert_eq!(event.payload().unwrap(), expected);
    }

    #[test]
    fn parses_removed_event_without_archived_at() {
        let event = stored(
            DEPLOYMENT_REMOVED,
            serde_json::json!({
                "deployment_id": "dpl-1",
                "release_id": "rls-1",
                "status": "archived",
                "activity_status": "archived",
                "error_status": "none",
                "target_status": "archived",
            }),
        );

        let expected = EventPayload::Removed(DeploymentRemovedEvent {
            deployment_id: "dpl-1".into(),
            release_id: "rls-1".into(),
            status: DplStatus::Archived,
            activity_status: DplActivity::Archived,
            error_status: DplErrStatus::None,
            target_status: DplTarget::Archived,
            archived_at: None,
        });
        assert_eq!(event.payload().unwrap(), expected);
    }

    #[test]
    fn rejects_unknown_event_type() {
        let event = stored("deployment.unknown", serde_json::json!({}));

        let result = event.payload();
        assert!(matches!(result, Err(EventsErr::UnknownEventTypeErr(_))));
    }

    #[test]
    fn rejects_data_that_does_not_match_its_type() {
        let data = serde_json::json!({"deployment_id": 1});
        let event = stored(DEPLOYMENT_DEPLOYED, data);

        let result = event.payload();
        assert!(matches!(result, Err(EventsErr::SerializationErr(_))));
    }
}
