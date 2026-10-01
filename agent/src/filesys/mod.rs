pub mod dir;
pub mod dirs;
pub mod errors;
pub mod file;
pub mod files;
pub mod path;
pub mod state_file;

// internal crates
pub use self::dir::Dir;
pub use self::errors::FileSysErr;
pub use self::file::File;
pub use self::path::PathExt;

/// Mode for agent-private files under the data root; ignored on Windows.
pub const PRIVATE_FILE_MODE: u32 = 0o600;

/// Mode for agent-private folders under the data root; ignored on Windows.
pub const PRIVATE_DIR_MODE: u32 = 0o700;

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
    /// Unix permission bits for created files; ignored on Windows (NTFS
    /// ACLs inherited from the parent directory own this concern).
    pub mode: Option<u32>,
}

impl WriteOptions {
    /// Overwrite existing files using atomic writes.
    pub const OVERWRITE_ATOMIC: Self = Self {
        overwrite: Overwrite::Allow,
        atomic: Atomic::Yes,
        mode: None,
    };

    /// Overwrite existing files, non-atomic.
    pub const OVERWRITE_NONATOMIC: Self = Self {
        overwrite: Overwrite::Allow,
        atomic: Atomic::No,
        mode: None,
    };

    /// Atomic write no overwrite.
    pub const ATOMIC: Self = Self {
        overwrite: Overwrite::Deny,
        atomic: Atomic::Yes,
        mode: None,
    };

    /// Overwrite existing files using atomic writes, owner-only
    /// ([`PRIVATE_FILE_MODE`]).
    pub const OVERWRITE_ATOMIC_PRIVATE: Self = Self {
        overwrite: Overwrite::Allow,
        atomic: Atomic::Yes,
        mode: Some(PRIVATE_FILE_MODE),
    };
}

/// Options for file append operations.
#[derive(Clone, Copy, Debug, Default)]
pub struct AppendOptions {
    pub sync: Sync,
    /// Unix permission bits applied when the append creates the file; ignored
    /// on Windows.
    pub mode: Option<u32>,
}

impl AppendOptions {
    /// Append with `fdatasync` for crash durability.
    pub const SYNC: Self = Self {
        sync: Sync::Yes,
        mode: None,
    };

    /// [`Self::SYNC`], creating the file owner-only ([`PRIVATE_FILE_MODE`]).
    pub const SYNC_PRIVATE: Self = Self {
        sync: Sync::Yes,
        mode: Some(PRIVATE_FILE_MODE),
    };
}
