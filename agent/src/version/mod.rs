pub const GIT_RELEASE_TAG_KEY: Option<&str> = option_env!("MIRU_AGENT_GIT_RELEASE_TAG");
pub const GIT_COMMIT_HASH_KEY: Option<&str> = option_env!("MIRU_AGENT_GIT_COMMIT_HASH");

#[derive(Debug, Clone)]
pub struct BuildInfo {
    pub version: String,
    pub commit: String,
}

pub fn build_info() -> BuildInfo {
    BuildInfo {
        version: GIT_RELEASE_TAG_KEY.unwrap_or("unknown").to_string(),
        commit: GIT_COMMIT_HASH_KEY.unwrap_or("unknown").to_string(),
    }
}
