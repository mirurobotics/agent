// standard crates
use std::collections::HashMap;

// internal crates
use miru_agent::cache::CacheEntry;
use miru_agent::disk::{Capacities, Layout, Storage};
use miru_agent::filesys::{dirs, files, WriteOptions};
use miru_agent::models::Deployment;

// external crates
use chrono::{DateTime, TimeDelta, Utc};

// ─── retry state reset on init ──────────────────────────────────────────────

/// Helper: builds a `CacheEntry` for a deployment, suitable for
/// pre-populating the on-disk deployments.json before `Storage::init`.
fn make_entry(dpl: Deployment, is_dirty: bool) -> CacheEntry<String, Deployment> {
    CacheEntry {
        key: dpl.id.clone(),
        value: dpl,
        is_dirty,
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
    files::write_json(&file, &map, WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();
}

pub mod reset_retry_state_on_init {
    use super::*;

    #[tokio::test]
    async fn resets_deployment_with_attempts() {
        let dir = dirs::temp("reset_attempts").unwrap();
        let layout = Layout::new(dir.to_dir());

        let dpl = Deployment {
            id: "dpl-dirty".to_string(),
            attempts: 5,
            ..Default::default()
        };
        seed_deployments(&layout, vec![make_entry(dpl, false)]).await;

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
        let dir = dirs::temp("reset_cooldown").unwrap();
        let layout = Layout::new(dir.to_dir());

        let mut dpl = Deployment {
            id: "dpl-cooldown".to_string(),
            ..Default::default()
        };
        dpl.set_cooldown(TimeDelta::hours(1));
        seed_deployments(&layout, vec![make_entry(dpl, false)]).await;

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
        let dir = dirs::temp("reset_skip_clean").unwrap();
        let layout = Layout::new(dir.to_dir());

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
        seed_deployments(
            &layout,
            vec![make_entry(clean, false), make_entry(dirty, false)],
        )
        .await;

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

    #[tokio::test]
    async fn preserves_dirty_flag_on_reset() {
        let dir = dirs::temp("reset_preserves_dirty").unwrap();
        let layout = Layout::new(dir.to_dir());

        let mut pending = Deployment {
            id: "dpl-dirty-pending".to_string(),
            attempts: 3,
            ..Default::default()
        };
        pending.set_cooldown(TimeDelta::hours(1));
        let clean = Deployment {
            id: "dpl-clean".to_string(),
            attempts: 0,
            cooldown_ends_at: DateTime::<Utc>::UNIX_EPOCH,
            ..Default::default()
        };
        seed_deployments(
            &layout,
            vec![make_entry(pending, true), make_entry(clean, false)],
        )
        .await;

        let (storage, _) = Storage::init(&layout, Capacities::default(), "dev".to_string())
            .await
            .unwrap();

        // retry state is reset but the pending push (dirty flag) survives
        let entry = storage
            .deployments
            .read_entry("dpl-dirty-pending".to_string())
            .await
            .unwrap();
        assert_eq!(entry.value.attempts, 0);
        assert!(entry.value.has_clean_retry_state());
        assert!(
            entry.is_dirty,
            "dirty flag should survive the retry state reset"
        );

        // the clean, non-dirty entry is untouched
        let entry = storage
            .deployments
            .read_entry("dpl-clean".to_string())
            .await
            .unwrap();
        assert!(entry.value.has_clean_retry_state());
        assert!(!entry.is_dirty, "clean entry should remain non-dirty");
    }
}
