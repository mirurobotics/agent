pub mod cached_file;
pub mod dir;
pub mod errors;
pub mod file;
pub mod path;

// internal crates
pub use self::dir::Dir;
pub use self::errors::FileSysErr;
pub use self::file::File;
pub use self::path::PathExt;

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
    No,
    #[default]
    Yes,
}

/// Whether a write should be followed by `fdatasync` to ensure the data
/// reaches stable storage before returning.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sync {
    No,
    #[default]
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

/// Options for file append operations.
#[derive(Clone, Copy, Debug, Default)]
pub struct AppendOptions {
    pub sync: Sync,
}

impl AppendOptions {
    /// Append with `fdatasync` for crash durability.
    pub const SYNC: Self = Self { sync: Sync::Yes };
}

/// Options for file copy operations.
#[derive(Clone, Copy, Debug, Default)]
pub struct CopyOptions {
    pub overwrite: Overwrite,
    pub sync: Sync,
}

impl CopyOptions {
    /// Allow overwriting, sync after copy.
    pub const OVERWRITE_SYNC: Self = Self {
        overwrite: Overwrite::Allow,
        sync: Sync::Yes,
    };

    /// Allow overwriting, no sync.
    pub const OVERWRITE: Self = Self {
        overwrite: Overwrite::Allow,
        sync: Sync::No,
    };
}
