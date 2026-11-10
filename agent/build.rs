// build.rs
use std::process::Command;

const GIT_COMMIT_HASH_KEY: &str = "MIRU_AGENT_GIT_COMMIT_HASH";
const BUILD_DATE_KEY: &str = "MIRU_AGENT_BUILD_DATE";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    watch_git_refs();

    // Without git (a source tarball, a container with no git), `version::COMMIT`
    // falls back to "unknown".
    if let Some(commit_hash) = git(&["rev-parse", "HEAD"]) {
        println!("cargo:rustc-env={GIT_COMMIT_HASH_KEY}={commit_hash}");
    }
    println!("cargo:rustc-env={BUILD_DATE_KEY}={}", build_date());

    // When HEAD is tagged (release build), verify the tag matches Cargo.toml version.
    // Prevents releasing a binary that reports the wrong version.
    if let Some(tag) = git(&["describe", "--exact-match", "--tags", "HEAD"]) {
        let tag_version = tag.strip_prefix('v').unwrap_or(&tag);
        let cargo_version = std::env::var("CARGO_PKG_VERSION").unwrap();
        assert_eq!(
            tag_version, cargo_version,
            "git tag 'v{tag_version}' does not match Cargo.toml version '{cargo_version}' — \
             update [workspace.package] version before releasing"
        );
    }
}

/// Reruns this script when HEAD moves, a branch moves or a tag is added.
/// Cargo treats a missing path as always changed, so only paths that exist
/// are watched; otherwise every build would rerun the script and recompile
/// the crate.
fn watch_git_refs() {
    // git prints these relative to the working directory or absolute, and
    // joining handles both. In a worktree they differ: HEAD is per worktree,
    // refs are shared.
    let (Some(cwd), Some(git_dir), Some(common_dir)) = (
        std::env::current_dir().ok(),
        git(&["rev-parse", "--git-dir"]),
        git(&["rev-parse", "--git-common-dir"]),
    ) else {
        return;
    };
    let git_dir = cwd.join(git_dir);
    let common_dir = cwd.join(common_dir);

    // directories are scanned recursively, so a new loose ref is seen too
    let watched = [
        git_dir.join("HEAD"),
        common_dir.join("packed-refs"),
        common_dir.join("refs").join("heads"),
        common_dir.join("refs").join("tags"),
    ];
    for path in watched.iter().filter(|p| p.exists()) {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

/// UTC build time, to the second. Honors `SOURCE_DATE_EPOCH` for reproducible
/// builds.
fn build_date() -> String {
    let date = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|secs| secs.trim().parse::<i64>().ok())
        .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
        .unwrap_or_else(chrono::Utc::now);
    date.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Runs git with `args` and returns its trimmed stdout, or `None` if git is
/// missing or the command fails.
fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    Some(stdout.trim().to_string())
}
