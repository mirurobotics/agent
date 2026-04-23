pub mod display;
pub mod errors;
pub mod install;
pub mod provision;

// internal crates
pub use self::errors::InstallErr;
use crate::filesys::{Dir, PathExt};
use crate::logs;

pub async fn init_installer_logging() -> Result<(logs::Guard, Dir), InstallErr> {
    let tmp_dir = Dir::create_temp_dir("miru-agent-installer-logs").await?;
    let options = logs::Options {
        // sending logs to stdout will interfere with the installer outputs
        stdout: false,
        log_dir: tmp_dir.path().to_path_buf(),
        ..Default::default()
    };
    let guard = logs::init(options);
    Ok((guard, tmp_dir))
}
