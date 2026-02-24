pub mod config_instance;
pub mod deployment;
pub mod device;
pub mod errors;
pub mod release;

pub use self::config_instance::ConfigInstance;
pub use self::deployment::Deployment;
pub use self::deployment::DplActivity;
pub use self::deployment::DplErrStatus;
pub use self::deployment::DplStatus;
pub use self::deployment::DplTarget;
pub use self::device::Device;
pub use self::device::DeviceStatus;
pub use self::release::Release;

pub trait Patch<PatchT> {
    fn patch(&mut self, patch: PatchT);
}
