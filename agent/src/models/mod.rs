pub mod config_instance;
pub mod deployment;
pub mod device;
pub mod errors;
pub mod file_rule;
pub mod git_commit;
pub mod release;
pub(crate) mod status;

// internal crates
pub use self::config_instance::CfgInstID;
pub use self::config_instance::ConfigInstance;
pub use self::deployment::Deployment;
pub use self::deployment::DeploymentID;
pub use self::deployment::DplActivity;
pub use self::deployment::DplErrStatus;
pub use self::deployment::DplStatus;
pub use self::deployment::DplTarget;
pub use self::device::Device;
pub use self::device::DeviceStatus;
pub use self::errors::ModelsErr;
pub use self::file_rule::FileRule;
pub use self::file_rule::FileRuleID;
pub use self::file_rule::FileRuleRetention;
pub use self::file_rule::FileRuleSource;
pub use self::file_rule::FileRuleUpload;
pub use self::file_rule::UploadCollectionID;
pub use self::git_commit::GitCommit;
pub use self::git_commit::GitCommitID;
pub use self::release::Release;
pub use self::release::ReleaseID;

pub trait Patch<PatchT> {
    fn patch(&mut self, patch: PatchT);
}
