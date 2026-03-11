// internal crates
use miru_agent::events::model::{
    Envelope, NewEvent, DEPLOYMENT_DEPLOYED_BETA1, DEPLOYMENT_REMOVED_BETA1,
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
        assert_eq!(DEPLOYMENT_DEPLOYED_BETA1, "deployment.deployed.beta1");
    }

    #[test]
    fn deployment_removed_type_string() {
        assert_eq!(DEPLOYMENT_REMOVED_BETA1, "deployment.removed.beta1");
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

        let event = NewEvent::deployment_deployed(&dpl);
        assert_eq!(event.event_type, DEPLOYMENT_DEPLOYED_BETA1);
    }

    #[test]
    fn includes_deployment_id_in_data() {
        let dpl = Deployment {
            id: "dpl-42".into(),
            activity_status: DplActivity::Deployed,
            target_status: DplTarget::Deployed,
            ..Default::default()
        };

        let event = NewEvent::deployment_deployed(&dpl);
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

        let event = NewEvent::deployment_deployed(&dpl);
        assert_eq!(event.data["activity_status"], "deployed");
    }

    #[test]
    fn includes_target_status() {
        let dpl = Deployment {
            id: "dpl-1".into(),
            target_status: DplTarget::Deployed,
            ..Default::default()
        };

        let event = NewEvent::deployment_deployed(&dpl);
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

        let event = NewEvent::deployment_deployed(&dpl);
        assert!(event.data["deployed_at"].is_string());
    }

    #[test]
    fn deployed_at_null_when_absent() {
        let dpl = Deployment {
            id: "dpl-1".into(),
            deployed_at: None,
            ..Default::default()
        };

        let event = NewEvent::deployment_deployed(&dpl);
        assert!(event.data["deployed_at"].is_null());
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

        let event = NewEvent::deployment_removed(&dpl);
        assert_eq!(event.event_type, DEPLOYMENT_REMOVED_BETA1);
    }

    #[test]
    fn includes_deployment_id_in_data() {
        let dpl = Deployment {
            id: "dpl-99".into(),
            activity_status: DplActivity::Archived,
            target_status: DplTarget::Archived,
            ..Default::default()
        };

        let event = NewEvent::deployment_removed(&dpl);
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

        let event = NewEvent::deployment_removed(&dpl);
        assert!(event.data["archived_at"].is_string());
    }

    #[test]
    fn archived_at_null_when_absent() {
        let dpl = Deployment {
            id: "dpl-1".into(),
            archived_at: None,
            ..Default::default()
        };

        let event = NewEvent::deployment_removed(&dpl);
        assert!(event.data["archived_at"].is_null());
    }
}

// ========================= ENVELOPE ========================= //

mod envelope {
    use super::*;

    #[test]
    fn serialization_roundtrip() {
        let envelope = Envelope {
            id: 7,
            event_type: "test.roundtrip".to_string(),
            occurred_at: Utc::now(),
            data: serde_json::json!({"deployment_id": "dpl-1"}),
        };

        let json = serde_json::to_string(&envelope).unwrap();
        let deserialized: Envelope = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, 7);
        assert_eq!(deserialized.event_type, "test.roundtrip");
        assert_eq!(deserialized.data["deployment_id"], "dpl-1");
    }

    #[test]
    fn type_field_serialized_as_type() {
        let envelope = Envelope {
            id: 1,
            event_type: "test.rename".to_string(),
            occurred_at: Utc::now(),
            data: serde_json::json!({}),
        };

        let json = serde_json::to_string(&envelope).unwrap();
        // The field is serialized as "type" not "event_type"
        assert!(json.contains("\"type\":\"test.rename\""));
    }
}
