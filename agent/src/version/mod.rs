pub const VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));
pub const COMMIT: &str = match option_env!("MIRU_AGENT_GIT_COMMIT_HASH") {
    Some(v) => v,
    None => "unknown",
};
