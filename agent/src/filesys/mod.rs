pub mod cached_file;
pub mod dir;
pub mod errors;
pub mod file;
pub mod path;

/// Whether an operation is allowed to overwrite an existing file or directory.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Overwrite {
    #[default]
    Deny,
    Allow,
}

/// Whether a write should be performed atomically (write to a temporary file,
/// then rename into place).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Atomic {
    #[default]
    No,
    Yes,
}

/// Options for file write operations.
#[derive(Clone, Copy, Debug, Default)]
pub struct WriteOptions {
    pub overwrite: Overwrite,
    pub atomic: Atomic,
}

impl WriteOptions {
    /// Overwrite existing files using atomic writes.
    pub const OVERWRITE_ATOMIC: Self = Self {
        overwrite: Overwrite::Allow,
        atomic: Atomic::Yes,
    };

    /// Overwrite existing files, non-atomic.
    pub const OVERWRITE: Self = Self {
        overwrite: Overwrite::Allow,
        atomic: Atomic::No,
    };

    /// Atomic write no overwrite.
    pub const ATOMIC: Self = Self {
        overwrite: Overwrite::Deny,
        atomic: Atomic::Yes,
    };
}
