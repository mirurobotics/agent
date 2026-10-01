pub mod dirs;
pub mod files;

// standard crates
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

// internal crates
use miru_agent::filesys::File;
#[cfg(unix)]
use miru_agent::filesys::{Dir, PathExt};

/// Path rooted at the host separator (`/a/b` on Unix, `\a\b` on Windows),
/// built from a `/`-separated string. A `/`-prefixed literal is a relative
/// path on Windows, so rooted fixture paths go through this helper. It is
/// absolute on Unix; on Windows it is rooted but has no drive prefix, so
/// `Path::is_absolute()` is false. `File::new` and `Dir::new` normalize
/// components themselves, so a `/`-rooted literal is fine for those; use this
/// only for a raw `PathBuf` that will be compared or displayed.
pub fn abs_path(path: &str) -> PathBuf {
    let mut out = PathBuf::from(std::path::MAIN_SEPARATOR_STR);
    for part in path.split('/').filter(|p| !p.is_empty()) {
        out.push(part);
    }
    out
}

/// A file that does not exist on any host.
pub fn missing_file() -> File {
    File::new("/nonexistent/definitely/not/here.bin")
}

/// Assert `file`'s permission bits (including setuid/setgid/sticky) equal `expected`.
#[cfg(unix)]
pub async fn assert_file_mode(file: &File, expected: u32) {
    // full path: the sibling `files` fixture module shadows the library's
    let perms = miru_agent::filesys::files::permissions(file).await.unwrap();
    let actual = perms.mode() & 0o7777;
    assert_eq!(
        actual,
        expected,
        "{:?}: mode {actual:o}, want {expected:o}",
        file.path()
    );
}

/// Assert `dir`'s permission bits (including setuid/setgid/sticky) equal `expected`.
#[cfg(unix)]
pub async fn assert_dir_mode(dir: &Dir, expected: u32) {
    let perms = miru_agent::filesys::dirs::permissions(dir).await.unwrap();
    let actual = perms.mode() & 0o7777;
    assert_eq!(
        actual,
        expected,
        "{:?}: mode {actual:o}, want {expected:o}",
        dir.path()
    );
}
