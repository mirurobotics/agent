// build.rs
use std::process::Command;

const GIT_COMMIT_HASH_KEY: &str = "MIRU_AGENT_GIT_COMMIT_HASH";

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

    // When HEAD is tagged (release build), verify the tag matches Cargo.toml version.
    // Prevents releasing a binary that reports the wrong version.
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
