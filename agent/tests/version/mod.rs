// internal crates
use miru_agent::version;

#[test]
fn test_version_info() {
    let info = version::build_info();
    // build.rs sets these from git, so they should be non-empty
    assert!(!info.version.is_empty(), "version should not be empty");
    assert!(!info.commit.is_empty(), "commit should not be empty");
    assert_ne!(info.version, "unknown", "build.rs should set the version");
    assert_ne!(info.commit, "unknown", "build.rs should set the commit");
}

#[test]
fn test_git_keys_set_by_build_script() {
    // build.rs injects these env vars at compile time via cargo:rustc-env
    assert!(
        version::GIT_RELEASE_TAG_KEY.is_some(),
        "build.rs should set GIT_RELEASE_TAG_KEY"
    );
    assert!(
        version::GIT_COMMIT_HASH_KEY.is_some(),
        "build.rs should set GIT_COMMIT_HASH_KEY"
    );
}
