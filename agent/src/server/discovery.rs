// internal crates
use crate::filesys::{self, files, Atomic, Overwrite, WriteOptions};
use crate::server::{auth::Token, errors::ServerErr};

// external crates
use serde::Serialize;

/// Version of the discovery file format that clients read.
pub const SCHEMA_VERSION: u32 = 1;

/// Discovery file contents. Not `Debug`, so the token cannot reach logs.
#[derive(Serialize)]
struct Discovery<'a> {
    schema_version: u32,
    port: u16,
    token: &'a str,
}

/// Atomically write the TCP port and bearer token for local device API
/// clients. On Unix the file mode is 0640 (owner read-write, group read); on
/// Windows the file inherits the `device-api` directory ACL.
pub async fn write(file: &filesys::File, port: u16, token: &Token) -> Result<(), ServerErr> {
    let discovery = Discovery {
        schema_version: SCHEMA_VERSION,
        port,
        token: token.expose(),
    };
    let opts = WriteOptions {
        overwrite: Overwrite::Allow,
        atomic: Atomic::Yes,
        mode: Some(0o640),
    };
    files::write_json(file, &discovery, opts).await?;
    Ok(())
}

/// Remove the discovery file; a missing file is not an error.
pub async fn remove(file: &filesys::File) -> Result<(), ServerErr> {
    files::delete(file).await?;
    Ok(())
}
