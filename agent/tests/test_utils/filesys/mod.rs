pub mod dirs;
pub mod files;

// standard crates
use std::path::PathBuf;

// internal crates
use miru_agent::filesys::File;

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
