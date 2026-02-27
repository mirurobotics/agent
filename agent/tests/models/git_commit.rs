// internal crates
use miru_agent::models::git_commit::GitCommit;
use openapi_client::models::{self as backend_client, GitCommit as BackendGitCommit};

// external crates
use chrono::{DateTime, Utc};
use serde_json::json;

// harness
use crate::models::harnesses::{serde_tests, ModelFixture, OptionalField, RequiredField};

// ─── fixture ─────────────────────────────────────────────────────────────────

impl ModelFixture for GitCommit {
    fn required_fields() -> Vec<RequiredField> {
        vec![
            RequiredField {
                key: "id",
                value: json!("gc_123"),
            },
            RequiredField {
                key: "sha",
                value: json!("abc123def456"),
            },
            RequiredField {
                key: "message",
                value: json!("fix: something"),
            },
            RequiredField {
                key: "commit_url",
                value: json!("https://github.com/org/repo/commit/abc123"),
            },
        ]
    }

    fn optional_fields() -> Vec<OptionalField> {
        vec![
            OptionalField {
                key: "repository_owner",
                value: json!("miru-hq"),
                default_value: json!(""),
            },
            OptionalField {
                key: "repository_name",
                value: json!("miru"),
                default_value: json!(""),
            },
            OptionalField {
                key: "repository_type",
                value: json!("github"),
                default_value: json!(""),
            },
            OptionalField {
                key: "repository_url",
                value: json!("https://github.com/miru-hq/miru"),
                default_value: json!(""),
            },
            OptionalField {
                key: "created_at",
                value: json!("2023-11-14T22:13:20Z"),
                default_value: json!("1970-01-01T00:00:00Z"),
            },
        ]
    }
}

serde_tests!(GitCommit);

#[test]
fn defaults() {
    let actual = GitCommit::default();
    let id = actual.id.clone();
    assert!(id.starts_with("unknown-"));
    let expected = GitCommit {
        id,
        sha: String::new(),
        message: String::new(),
        repository_owner: String::new(),
        repository_name: String::new(),
        repository_type: String::new(),
        repository_url: String::new(),
        commit_url: String::new(),
        created_at: DateTime::<Utc>::UNIX_EPOCH,
    };
    assert_eq!(actual, expected);
}

// ─── model-specific tests ────────────────────────────────────────────────────

#[test]
fn from_backend() {
    let now = Utc::now();
    let backend_gc = BackendGitCommit {
        object: backend_client::git_commit::Object::GitCommit,
        id: "gc_123".to_string(),
        sha: "abc123def456".to_string(),
        message: "fix: something".to_string(),
        repository_owner: "miru-hq".to_string(),
        repository_name: "miru".to_string(),
        repository_type: backend_client::GitRepositoryType::GIT_REPO_TYPE_GITHUB,
        repository_url: "https://github.com/miru-hq/miru".to_string(),
        commit_url: "https://github.com/miru-hq/miru/commit/abc123".to_string(),
        created_at: now.to_rfc3339(),
    };

    let gc: GitCommit = backend_gc.into();

    assert!(gc.created_at > DateTime::<Utc>::UNIX_EPOCH);
    let expected = GitCommit {
        id: "gc_123".to_string(),
        sha: "abc123def456".to_string(),
        message: "fix: something".to_string(),
        repository_owner: "miru-hq".to_string(),
        repository_name: "miru".to_string(),
        repository_type: "github".to_string(),
        repository_url: "https://github.com/miru-hq/miru".to_string(),
        commit_url: "https://github.com/miru-hq/miru/commit/abc123".to_string(),
        created_at: now,
    };
    assert_eq!(gc, expected);
}

#[test]
fn from_backend_invalid_date() {
    let backend_gc = BackendGitCommit {
        object: backend_client::git_commit::Object::GitCommit,
        id: "gc_bad".to_string(),
        sha: "deadbeef".to_string(),
        message: "bad date".to_string(),
        repository_owner: "owner".to_string(),
        repository_name: "repo".to_string(),
        repository_type: backend_client::GitRepositoryType::GIT_REPO_TYPE_GITLAB,
        repository_url: "https://gitlab.com/owner/repo".to_string(),
        commit_url: "https://gitlab.com/owner/repo/-/commit/deadbeef".to_string(),
        created_at: "not-a-date".to_string(),
    };

    let gc: GitCommit = backend_gc.into();
    assert_eq!(gc.id, "gc_bad");
    assert_eq!(gc.repository_type, "gitlab");
    assert_eq!(gc.created_at, DateTime::<Utc>::UNIX_EPOCH);
}
