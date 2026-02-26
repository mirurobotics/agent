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

#[test]
fn test_build_date_set_by_build_script() {
    // build.rs sets MIRU_AGENT_BUILD_DATE to YYYY-MM-DD
    assert_ne!(version::BUILD_DATE, "unknown");
    assert_eq!(version::BUILD_DATE.len(), 10);
    assert!(version::BUILD_DATE.starts_with("20"));
    assert_eq!(version::BUILD_DATE.as_bytes()[4], b'-');
    assert_eq!(version::BUILD_DATE.as_bytes()[7], b'-');
}

#[test]
fn test_rust_version_is_semver() {
    assert!(!version::RUST_VERSION.is_empty());
    assert!(version::RUST_VERSION.contains('.'));
}

#[test]
fn test_os_is_not_empty() {
    assert!(!version::OS.is_empty());
}

#[test]
fn test_arch_is_not_empty() {
    assert!(!version::ARCH.is_empty());
}

#[test]
fn test_api_version_is_not_empty() {
    assert!(!version::api_version().is_empty());
}

#[test]
fn test_api_git_commit_is_not_empty() {
    assert!(!version::api_git_commit().is_empty());
}
