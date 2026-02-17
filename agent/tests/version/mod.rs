// internal crates
use miru_agent::version;

#[test]
fn test_version_matches_cargo_pkg() {
    assert_eq!(version::VERSION, concat!("v", env!("CARGO_PKG_VERSION")));
}

#[test]
fn test_commit_set_by_build_script() {
    // In a git repo, COMMIT should be a 40-char hex SHA
    assert_ne!(version::COMMIT, "unknown");
    assert_eq!(version::COMMIT.len(), 40);
    assert!(version::COMMIT.chars().all(|c| c.is_ascii_hexdigit()));
}
