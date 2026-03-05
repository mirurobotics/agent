// internal crates
use miru_agent::cache::CacheEntry;
use miru_agent::models::{Deployment, DplActivity, DplErrStatus};
use miru_agent::storage::deployments::is_dirty;

// external crates
use chrono::Utc;

pub mod is_dirty_func {
    use super::*;

    #[tokio::test]
    async fn no_changes() {
        let deployment = Deployment {
            ..Default::default()
        };
        let entry = CacheEntry {
            key: deployment.id.clone(),
            value: deployment.clone(),
            is_dirty: false,
            created_at: Utc::now(),
            last_accessed: Utc::now(),
        };
        let old = Some(&entry);
        assert!(!is_dirty(old, &deployment));
    }

    #[tokio::test]
    async fn previous_is_none() {
        let deployment = Deployment {
            ..Default::default()
        };
        assert!(is_dirty(None, &deployment));
    }

    #[tokio::test]
    async fn previously_dirty() {
        let deployment = Deployment {
            ..Default::default()
        };
        let entry = CacheEntry {
            key: deployment.id.clone(),
            value: deployment.clone(),
            is_dirty: true,
            created_at: Utc::now(),
            last_accessed: Utc::now(),
        };
        let old = Some(&entry);
        assert!(is_dirty(old, &deployment));
    }

    #[tokio::test]
    async fn activity_status_changed() {
        let old_deployment = Deployment {
            activity_status: DplActivity::Queued,
            ..Default::default()
        };
        let new_deployment = Deployment {
            id: old_deployment.id.clone(),
            activity_status: DplActivity::Deployed,
            ..Default::default()
        };
        let entry = CacheEntry {
            key: old_deployment.id.clone(),
            value: old_deployment.clone(),
            is_dirty: false,
            created_at: Utc::now(),
            last_accessed: Utc::now(),
        };
        let old = Some(&entry);
        assert!(is_dirty(old, &new_deployment));
    }

    #[tokio::test]
    async fn error_status_changed() {
        let old_deployment = Deployment {
            error_status: DplErrStatus::None,
            ..Default::default()
        };
        let new_deployment = Deployment {
            id: old_deployment.id.clone(),
            error_status: DplErrStatus::Retrying,
            ..Default::default()
        };
        let entry = CacheEntry {
            key: old_deployment.id.clone(),
            value: old_deployment.clone(),
            is_dirty: false,
            created_at: Utc::now(),
            last_accessed: Utc::now(),
        };
        let old = Some(&entry);
        assert!(is_dirty(old, &new_deployment));
    }

    #[tokio::test]
    async fn deployed_at_changed() {
        let old_deployment = Deployment {
            ..Default::default()
        };
        let new_deployment = Deployment {
            id: old_deployment.id.clone(),
            deployed_at: Some(Utc::now()),
            ..Default::default()
        };
        let entry = CacheEntry {
            key: old_deployment.id.clone(),
            value: old_deployment.clone(),
            is_dirty: false,
            created_at: Utc::now(),
            last_accessed: Utc::now(),
        };
        let old = Some(&entry);
        assert!(is_dirty(old, &new_deployment));
    }

    #[tokio::test]
    async fn archived_at_changed() {
        let old_deployment = Deployment {
            ..Default::default()
        };
        let new_deployment = Deployment {
            id: old_deployment.id.clone(),
            archived_at: Some(Utc::now()),
            ..Default::default()
        };
        let entry = CacheEntry {
            key: old_deployment.id.clone(),
            value: old_deployment.clone(),
            is_dirty: false,
            created_at: Utc::now(),
            last_accessed: Utc::now(),
        };
        let old = Some(&entry);
        assert!(is_dirty(old, &new_deployment));
    }
}
