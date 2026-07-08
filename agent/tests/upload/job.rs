// internal crates
use miru_agent::filesys::File;
use miru_agent::upload::UploadJob;

// external crates
use chrono::{Duration, Utc};

fn make_job(upload_rule_id: &str, path: &str, digest: &str) -> UploadJob {
    UploadJob {
        file: File::new(path),
        size: 42,
        digest: digest.to_string(),
        mtime: Utc::now(),
        upload_rule_id: upload_rule_id.to_string(),
        deployment_id: "dpl_1".to_string(),
        release_id: "rls_1".to_string(),
    }
}

mod dedup_key {
    use super::*;

    #[test]
    fn equal_for_same_rule_path_digest() {
        let a = make_job("rule_1", "/data/output.log", "sha256:abc");
        let mut b = make_job("rule_1", "/data/output.log", "sha256:abc");
        b.size = 99;
        b.mtime = a.mtime + Duration::seconds(10);
        b.deployment_id = "dpl_2".to_string();
        b.release_id = "rls_2".to_string();

        assert_eq!(a.dedup_key(), b.dedup_key());
    }

    #[test]
    fn differs_when_any_component_differs() {
        let base = make_job("rule_1", "/data/output.log", "sha256:abc");

        let diff_rule = make_job("rule_2", "/data/output.log", "sha256:abc");
        assert_ne!(base.dedup_key(), diff_rule.dedup_key());

        let diff_path = make_job("rule_1", "/data/other.log", "sha256:abc");
        assert_ne!(base.dedup_key(), diff_path.dedup_key());

        let diff_digest = make_job("rule_1", "/data/output.log", "sha256:def");
        assert_ne!(base.dedup_key(), diff_digest.dedup_key());
    }

    #[test]
    fn equal_size_and_mtime_alone_do_not_make_keys_equal() {
        let a = make_job("rule_1", "/data/output.log", "sha256:abc");
        let mut b = make_job("rule_2", "/data/output.log", "sha256:abc");
        b.size = a.size;
        b.mtime = a.mtime;

        assert_ne!(a.dedup_key(), b.dedup_key());
    }
}
