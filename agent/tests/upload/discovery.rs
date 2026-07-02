// standard crates
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

// internal crates
use miru_agent::filesys::{self, PathExt, WriteOptions};
use miru_agent::models::{UploadRule, UploadRuleSource};
use miru_agent::upload::discovery::{
    discover_blocking, is_ready, observe, passes_finalization_markers, Candidate, Observation,
};

// external crates
use chrono::{DateTime, TimeDelta, Utc};

pub fn make_rule(id: &str, glob: &str, stability_window_secs: i32) -> UploadRule {
    UploadRule {
        id: id.to_string(),
        source: UploadRuleSource {
            glob: glob.to_string(),
            stability_window_secs,
        },
        ..Default::default()
    }
}

pub mod observe_fn {
    use super::*;

    #[test]
    fn first_sighting_starts_the_watermark() {
        let now = Utc::now();
        let modified_at = now - TimeDelta::seconds(30);

        let observation = observe(None, 42, modified_at, now);

        let expected = Observation {
            size: 42,
            modified_at,
            unchanged_since: now,
        };
        assert_eq!(observation, expected);
    }

    #[test]
    fn unchanged_stat_preserves_the_watermark() {
        let first_seen = Utc::now();
        let modified_at = first_seen - TimeDelta::seconds(30);
        let prev = observe(None, 42, modified_at, first_seen);

        let now = first_seen + TimeDelta::seconds(120);
        let observation = observe(Some(&prev), 42, modified_at, now);

        assert_eq!(observation, prev);
    }

    #[test]
    fn size_change_resets_the_watermark() {
        let first_seen = Utc::now();
        let modified_at = first_seen - TimeDelta::seconds(30);
        let prev = observe(None, 42, modified_at, first_seen);

        let now = first_seen + TimeDelta::seconds(120);
        let observation = observe(Some(&prev), 43, modified_at, now);

        let expected = Observation {
            size: 43,
            modified_at,
            unchanged_since: now,
        };
        assert_eq!(observation, expected);
    }

    #[test]
    fn mtime_change_resets_the_watermark() {
        let first_seen = Utc::now();
        let modified_at = first_seen - TimeDelta::seconds(30);
        let prev = observe(None, 42, modified_at, first_seen);

        let now = first_seen + TimeDelta::seconds(120);
        let new_modified_at = modified_at + TimeDelta::seconds(60);
        let observation = observe(Some(&prev), 42, new_modified_at, now);

        let expected = Observation {
            size: 42,
            modified_at: new_modified_at,
            unchanged_since: now,
        };
        assert_eq!(observation, expected);
    }
}

pub mod is_ready_fn {
    use super::*;

    fn observation_unchanged_for(secs: i64) -> (Observation, DateTime<Utc>) {
        let now = Utc::now();
        let observation = Observation {
            size: 42,
            modified_at: now - TimeDelta::seconds(secs + 30),
            unchanged_since: now - TimeDelta::seconds(secs),
        };
        (observation, now)
    }

    #[test]
    fn not_ready_within_the_window() {
        let (observation, now) = observation_unchanged_for(59);
        assert!(!is_ready(&observation, 60, now));
    }

    #[test]
    fn ready_at_the_window_boundary() {
        let (observation, now) = observation_unchanged_for(60);
        assert!(is_ready(&observation, 60, now));
    }

    #[test]
    fn ready_beyond_the_window() {
        let (observation, now) = observation_unchanged_for(120);
        assert!(is_ready(&observation, 60, now));
    }

    #[test]
    fn zero_window_is_immediately_ready() {
        let (observation, now) = observation_unchanged_for(0);
        assert!(is_ready(&observation, 0, now));
    }

    #[test]
    fn negative_window_is_treated_as_zero() {
        let (observation, now) = observation_unchanged_for(0);
        assert!(is_ready(&observation, -10, now));
    }
}

pub mod passes_finalization_markers_fn {
    use super::*;

    #[test]
    fn passes_without_marker_detection() {
        assert!(passes_finalization_markers(Path::new("/data/logs/a.mcap")));
    }
}

pub mod discover_blocking_fn {
    use super::*;

    async fn write_file(dir: &filesys::Dir, name: &str, content: &str) -> filesys::File {
        let file = dir.file(name);
        file.write_string(content, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
        file
    }

    fn glob_for(dir: &filesys::Dir, pattern: &str) -> String {
        format!("{}/{}", dir.path().display(), pattern)
    }

    #[tokio::test]
    async fn stats_matching_files() {
        let dir = filesys::Dir::create_temp_dir("discover-stats")
            .await
            .unwrap();
        let file = write_file(&dir, "a.log", "hello").await;

        let rules = vec![make_rule("rule_1", &glob_for(&dir, "*.log"), 60)];
        let candidates = discover_blocking(&rules);

        let modified_at =
            DateTime::<Utc>::from(std::fs::metadata(file.path()).unwrap().modified().unwrap());
        let expected = Candidate {
            path: file.path().clone(),
            size: "hello".len() as u64,
            modified_at,
            stability_window_secs: 60,
        };
        assert_eq!(candidates, vec![expected]);
    }

    #[tokio::test]
    async fn skips_directories() {
        let dir = filesys::Dir::create_temp_dir("discover-dirs")
            .await
            .unwrap();
        dir.subdir("nested.log").create().await.unwrap();
        write_file(&dir, "a.log", "hello").await;

        let rules = vec![make_rule("rule_1", &glob_for(&dir, "*.log"), 0)];
        let candidates = discover_blocking(&rules);

        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].path.ends_with("a.log"));
    }

    #[tokio::test]
    async fn dedupes_overlapping_globs_with_the_smallest_window() {
        let dir = filesys::Dir::create_temp_dir("discover-dedupe")
            .await
            .unwrap();
        write_file(&dir, "a.log", "hello").await;

        let rules = vec![
            make_rule("rule_1", &glob_for(&dir, "*.log"), 60),
            make_rule("rule_2", &glob_for(&dir, "a.*"), 30),
        ];
        let candidates = discover_blocking(&rules);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].stability_window_secs, 30);
    }

    #[tokio::test]
    async fn skips_invalid_glob_patterns() {
        let dir = filesys::Dir::create_temp_dir("discover-invalid")
            .await
            .unwrap();
        write_file(&dir, "a.log", "hello").await;

        let rules = vec![
            make_rule("rule_1", "[", 0),
            make_rule("rule_2", &glob_for(&dir, "*.log"), 0),
        ];
        let candidates = discover_blocking(&rules);

        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].path.ends_with("a.log"));
    }

    #[tokio::test]
    async fn skips_unreadable_directories() {
        let dir = filesys::Dir::create_temp_dir("discover-unreadable")
            .await
            .unwrap();
        write_file(&dir, "a.log", "hello").await;
        let locked = dir.subdir("locked");
        locked.create().await.unwrap();
        write_file(&locked, "b.log", "hidden").await;
        locked
            .set_permissions(std::fs::Permissions::from_mode(0o000))
            .await
            .unwrap();

        let rules = vec![
            make_rule("rule_1", &glob_for(&dir, "locked/*.log"), 0),
            make_rule("rule_2", &glob_for(&dir, "*.log"), 0),
        ];
        let candidates = discover_blocking(&rules);

        // restore permissions so the temp dir can be cleaned up
        locked
            .set_permissions(std::fs::Permissions::from_mode(0o755))
            .await
            .unwrap();

        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].path.ends_with("a.log"));
    }

    #[tokio::test]
    async fn skips_dangling_symlinks() {
        let dir = filesys::Dir::create_temp_dir("discover-dangling")
            .await
            .unwrap();
        write_file(&dir, "a.log", "hello").await;
        std::os::unix::fs::symlink(
            dir.path().join("missing-target"),
            dir.path().join("dangling.log"),
        )
        .unwrap();

        let rules = vec![make_rule("rule_1", &glob_for(&dir, "*.log"), 0)];
        let candidates = discover_blocking(&rules);

        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].path.ends_with("a.log"));
    }

    #[tokio::test]
    async fn no_rules_yields_no_candidates() {
        let candidates = discover_blocking(&[]);
        assert!(candidates.is_empty());
    }

    #[tokio::test]
    async fn unmatched_glob_yields_no_candidates() {
        let dir = filesys::Dir::create_temp_dir("discover-empty")
            .await
            .unwrap();

        let rules = vec![make_rule("rule_1", &glob_for(&dir, "*.log"), 0)];
        let candidates = discover_blocking(&rules);

        assert!(candidates.is_empty());
    }
}
