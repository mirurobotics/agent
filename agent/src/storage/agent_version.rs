// internal crates
use crate::filesys::{self, PathExt, WriteOptions};
use crate::storage::errors::StorageErr;

// external crates
use serde::{Deserialize, Serialize};

/// Marker file written to `/var/lib/miru/agent_version.json` recording the
/// agent version of the binary that last successfully bootstrapped the
/// on-disk state. Read once at boot and written once on rebootstrap.
///
/// Unlike `storage::Device` (a `ConcurrentCachedFile`-based actor), this is
/// a plain `Serialize + Deserialize` struct with thin file helpers, mirroring
/// the simpler `storage::Settings` pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentVersion {
    pub version: String,
}

pub async fn read(file: &filesys::File) -> Result<Option<AgentVersion>, StorageErr> {
    if !file.exists() {
        return Ok(None);
    }
    Ok(Some(file.read_json::<AgentVersion>().await?))
}

pub async fn write(file: &filesys::File, marker: &AgentVersion) -> Result<(), StorageErr> {
    file.write_json(marker, WriteOptions::OVERWRITE_ATOMIC)
        .await?;
    Ok(())
}
