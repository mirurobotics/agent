// build.rs
use std::process::Command;

pub const GIT_RELEASE_TAG_KEY: &str = "MIRU_AGENT_GIT_RELEASE_TAG";
pub const GIT_COMMIT_HASH_KEY: &str = "MIRU_AGENT_GIT_COMMIT_HASH";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/");

    // Get the latest git commit hash
    let commit_hash = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("Failed to execute git rev-parse")
        .stdout;
    let commit_hash = String::from_utf8(commit_hash).expect("Invalid UTF-8 in git output");
    println!(
        "cargo:rustc-env={}={}",
        GIT_COMMIT_HASH_KEY,
        commit_hash.trim()
    );

    // Get the latest tag
    let release_tag = Command::new("git")
        .args(["describe", "--tags", "--abbrev=0"])
        .output()
        .expect("Failed to execute git describe")
        .stdout;
    let release_tag = String::from_utf8(release_tag).expect("Invalid UTF-8 in git output");
    println!(
        "cargo:rustc-env={}={}",
        GIT_RELEASE_TAG_KEY,
        release_tag.trim()
    );

    // When HEAD is tagged (release build), verify the tag matches Cargo.toml version.
    // Skipped during development when HEAD is not an exact tag.
    let exact_tag = Command::new("git")
        .args(["describe", "--exact-match", "--tags", "HEAD"])
        .output()
        .expect("Failed to execute git describe");
    if exact_tag.status.success() {
        let tag = String::from_utf8(exact_tag.stdout).expect("Invalid UTF-8 in git output");
        let tag_version = tag.trim().strip_prefix('v').unwrap_or(tag.trim());
        let cargo_version = std::env::var("CARGO_PKG_VERSION").unwrap();
        assert_eq!(
            tag_version, cargo_version,
            "git tag 'v{tag_version}' does not match Cargo.toml version '{cargo_version}' — \
             update [workspace.package] version before releasing"
        );
    }
}
