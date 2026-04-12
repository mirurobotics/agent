// standard crates
use std::collections::HashMap;

// internal crates
use miru_agent::cache::CacheEntry;
use miru_agent::filesys::{self, WriteOptions};
use miru_agent::models::{Deployment, DplActivity, DplErrStatus};
use miru_agent::storage::deployments::is_dirty;
use miru_agent::storage::{Capacities, Layout, Storage};

// external crates
use chrono::{DateTime, TimeDelta, Utc};

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

// ─── retry state reset on init ──────────────────────────────────────────────

/// Helper: builds a `CacheEntry` for a deployment, suitable for
/// pre-populating the on-disk deployments.json before `Storage::init`.
fn make_entry(dpl: Deployment) -> CacheEntry<String, Deployment> {
    CacheEntry {
        key: dpl.id.clone(),
        value: dpl,
        is_dirty: false,
        created_at: Utc::now(),
        last_accessed: Utc::now(),
    }
}

/// Writes a pre-populated deployments.json into the layout so that
/// `Storage::init` loads it and runs `reset_deployment_retry_state`.
async fn seed_deployments(layout: &Layout, entries: Vec<CacheEntry<String, Deployment>>) {
    let file = layout.deployments();
    let mut map: HashMap<String, CacheEntry<String, Deployment>> = HashMap::new();
    for entry in entries {
        map.insert(entry.key.clone(), entry);
    }
    file.write_json(&map, WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();
}

pub mod reset_retry_state_on_init {
    use super::*;

    #[tokio::test]
    async fn resets_deployment_with_attempts() {
        let dir = filesys::Dir::create_temp_dir("reset_attempts")
            .await
            .unwrap();
        let layout = Layout::new(dir);

        let dpl = Deployment {
            id: "dpl-dirty".to_string(),
            attempts: 5,
            ..Default::default()
        };
        seed_deployments(&layout, vec![make_entry(dpl)]).await;

        let (storage, _) = Storage::init(&layout, Capacities::default(), "dev".to_string())
            .await
            .unwrap();

        let loaded = storage
            .deployments
            .read_optional("dpl-dirty".to_string())
            .await
            .unwrap();
        let dpl = loaded.expect("deployment should exist");
        assert_eq!(dpl.attempts, 0, "attempts should be reset to 0");
        assert!(dpl.has_clean_retry_state());
    }

    #[tokio::test]
    async fn resets_deployment_with_active_cooldown() {
        let dir = filesys::Dir::create_temp_dir("reset_cooldown")
            .await
            .unwrap();
        let layout = Layout::new(dir);

        let mut dpl = Deployment {
            id: "dpl-cooldown".to_string(),
            ..Default::default()
        };
        dpl.set_cooldown(TimeDelta::hours(1));
        seed_deployments(&layout, vec![make_entry(dpl)]).await;

        let (storage, _) = Storage::init(&layout, Capacities::default(), "dev".to_string())
            .await
            .unwrap();

        let loaded = storage
            .deployments
            .read_optional("dpl-cooldown".to_string())
            .await
            .unwrap();
        let dpl = loaded.expect("deployment should exist");
        assert!(!dpl.is_in_cooldown(), "cooldown should be cleared");
        assert!(dpl.has_clean_retry_state());
    }

    #[tokio::test]
    async fn skips_clean_deployments() {
        let dir = filesys::Dir::create_temp_dir("reset_skip_clean")
            .await
            .unwrap();
        let layout = Layout::new(dir);

        let clean = Deployment {
            id: "dpl-clean".to_string(),
            attempts: 0,
            cooldown_ends_at: DateTime::<Utc>::UNIX_EPOCH,
            ..Default::default()
        };
        let mut dirty = Deployment {
            id: "dpl-dirty".to_string(),
            attempts: 3,
            ..Default::default()
        };
        dirty.set_cooldown(TimeDelta::hours(1));
        seed_deployments(&layout, vec![make_entry(clean), make_entry(dirty)]).await;

        let (storage, _) = Storage::init(&layout, Capacities::default(), "dev".to_string())
            .await
            .unwrap();

        // dirty deployment should be reset
        let loaded_dirty = storage
            .deployments
            .read_optional("dpl-dirty".to_string())
            .await
            .unwrap();
        let dpl = loaded_dirty.expect("dirty deployment should exist");
        assert_eq!(dpl.attempts, 0);
        assert!(dpl.has_clean_retry_state());

        // clean deployment should still be clean (unchanged)
        let loaded_clean = storage
            .deployments
            .read_optional("dpl-clean".to_string())
            .await
            .unwrap();
        let dpl = loaded_clean.expect("clean deployment should exist");
        assert_eq!(dpl.attempts, 0);
        assert!(dpl.has_clean_retry_state());
    }
}
