// internal crates
use miru_agent::events::model::{Event, EventArgs, DEPLOYMENT_DEPLOYED, DEPLOYMENT_REMOVED};
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

// ========================= DEPLOYMENT DEPLOYED ========================= //

mod deployment_deployed {
    use super::*;

    #[test]
    fn creates_correct_event_type() {
        let t = fixed_time();
        let dpl = Deployment {
            id: "dpl-1".into(),
            activity_status: DplActivity::Deployed,
            error_status: DplErrStatus::None,
            target_status: DplTarget::Deployed,
            deployed_at: Some(t),
            ..Default::default()
        };

        let event = EventArgs::deployed(&dpl).unwrap();
        assert_eq!(event.event_type, DEPLOYMENT_DEPLOYED);
    }

    #[test]
    fn includes_deployment_id_in_data() {
        let dpl = Deployment {
            id: "dpl-42".into(),
            activity_status: DplActivity::Deployed,
            target_status: DplTarget::Deployed,
            ..Default::default()
        };

        let event = EventArgs::deployed(&dpl).unwrap();
        assert_eq!(event.data["deployment_id"], "dpl-42");
    }

    #[test]
    fn includes_activity_status() {
        let dpl = Deployment {
            id: "dpl-1".into(),
            activity_status: DplActivity::Deployed,
            target_status: DplTarget::Deployed,
            ..Default::default()
        };

        let event = EventArgs::deployed(&dpl).unwrap();
        assert_eq!(event.data["activity_status"], "deployed");
    }

    #[test]
    fn includes_target_status() {
        let dpl = Deployment {
            id: "dpl-1".into(),
            target_status: DplTarget::Deployed,
            ..Default::default()
        };

        let event = EventArgs::deployed(&dpl).unwrap();
        assert_eq!(event.data["target_status"], "deployed");
    }

    #[test]
    fn includes_deployed_at_when_present() {
        let t = fixed_time();
        let dpl = Deployment {
            id: "dpl-1".into(),
            deployed_at: Some(t),
            ..Default::default()
        };

        let event = EventArgs::deployed(&dpl).unwrap();
        assert!(event.data["deployed_at"].is_string());
    }

    #[test]
    fn deployed_at_null_when_absent() {
        let dpl = Deployment {
            id: "dpl-1".into(),
            deployed_at: None,
            ..Default::default()
        };

        let event = EventArgs::deployed(&dpl).unwrap();
        assert!(event.data["deployed_at"].is_null());
    }

    #[test]
    fn serializes_non_deployed_activity_without_validation() {
        // deployed() is a factory, not a validator — it serializes
        // whatever activity_status is present without checking it
        let dpl = Deployment {
            id: "dpl-1".into(),
            activity_status: DplActivity::Queued,
            target_status: DplTarget::Deployed,
            ..Default::default()
        };

        let event = EventArgs::deployed(&dpl).unwrap();
        assert_eq!(event.event_type, DEPLOYMENT_DEPLOYED);
        assert_eq!(event.data["activity_status"], "queued");
    }
}

// ========================= DEPLOYMENT REMOVED ========================= //

mod deployment_removed {
    use super::*;

    #[test]
    fn creates_correct_event_type() {
        let dpl = Deployment {
            id: "dpl-1".into(),
            activity_status: DplActivity::Archived,
            target_status: DplTarget::Archived,
            ..Default::default()
        };

        let event = EventArgs::removed(&dpl).unwrap();
        assert_eq!(event.event_type, DEPLOYMENT_REMOVED);
    }

    #[test]
    fn includes_deployment_id_in_data() {
        let dpl = Deployment {
            id: "dpl-99".into(),
            activity_status: DplActivity::Archived,
            target_status: DplTarget::Archived,
            ..Default::default()
        };

        let event = EventArgs::removed(&dpl).unwrap();
        assert_eq!(event.data["deployment_id"], "dpl-99");
    }

    #[test]
    fn includes_archived_at_when_present() {
        let t = fixed_time();
        let dpl = Deployment {
            id: "dpl-1".into(),
            archived_at: Some(t),
            ..Default::default()
        };

        let event = EventArgs::removed(&dpl).unwrap();
        assert!(event.data["archived_at"].is_string());
    }

    #[test]
    fn archived_at_null_when_absent() {
        let dpl = Deployment {
            id: "dpl-1".into(),
            archived_at: None,
            ..Default::default()
        };

        let event = EventArgs::removed(&dpl).unwrap();
        assert!(event.data["archived_at"].is_null());
    }

    #[test]
    fn serializes_non_archived_activity_without_validation() {
        let dpl = Deployment {
            id: "dpl-1".into(),
            activity_status: DplActivity::Deployed,
            target_status: DplTarget::Archived,
            ..Default::default()
        };

        let event = EventArgs::removed(&dpl).unwrap();
        assert_eq!(event.event_type, DEPLOYMENT_REMOVED);
        assert_eq!(event.data["activity_status"], "deployed");
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
