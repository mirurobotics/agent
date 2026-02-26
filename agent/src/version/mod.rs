pub const VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));
pub const COMMIT: &str = match option_env!("MIRU_AGENT_GIT_COMMIT_HASH") {
    Some(v) => v,
    None => "unknown",
};
pub const RUST_VERSION: &str = env!("CARGO_PKG_RUST_VERSION");
pub const BUILD_DATE: &str = match option_env!("MIRU_AGENT_BUILD_DATE") {
    Some(v) => v,
    None => "unknown",
};
pub const OS: &str = std::env::consts::OS;
pub const ARCH: &str = std::env::consts::ARCH;

pub fn api_version() -> String {
    openapi_server::models::ApiVersion::API_VERSION.to_string()
}

pub fn api_git_commit() -> String {
    openapi_server::models::ApiGitCommit::API_GIT_COMMIT.to_string()
}

pub fn format() -> String {
    format!("Version: {}\nCommit: {}", VERSION, COMMIT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_contains_version_and_commit() {
        let output = format();
        assert!(output.starts_with("Version: v"));
        assert!(output.contains("\nCommit: "));
    }
}
