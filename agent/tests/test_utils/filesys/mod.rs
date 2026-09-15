pub mod dirs;
pub mod files;

// standard crates
use std::path::PathBuf;

// internal crates
use miru_agent::filesys::{Dir, File};

/// Path rooted at the host separator (`/a/b` on Unix, `\a\b` on Windows),
/// built from a `/`-separated string. A `/`-prefixed literal is a relative
/// path on Windows, so rooted fixture paths go through this helper. It is
/// absolute on Unix; on Windows it is rooted but has no drive prefix, so
/// `Path::is_absolute()` is false — use it only for paths that are never
/// resolved or canonicalized.
pub fn abs_path(path: &str) -> PathBuf {
    let mut out = PathBuf::from(std::path::MAIN_SEPARATOR_STR);
    for part in path.split('/').filter(|p| !p.is_empty()) {
        out.push(part);
    }
    out
}

pub fn abs_file(path: &str) -> File {
    File::new(abs_path(path))
}

pub fn abs_dir(path: &str) -> Dir {
    Dir::new(abs_path(path))
}

/// A file that does not exist on any host.
pub fn missing_file() -> File {
    abs_file("nonexistent/definitely/not/here.bin")
}
