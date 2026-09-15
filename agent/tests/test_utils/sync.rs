// standard crates
use std::sync::Arc;

// internal crates
use super::filesys::files as test_files;
use super::http_client::MockClient;
use miru_agent::authn::token_mngr::TokenFile;
use miru_agent::authn::{Token, TokenManager};
use miru_agent::disk::{
    self, CfgInstContent, CfgInstStor, CfgInsts, Deployments, FileRules, GitCommits, Releases,
    Storage,
};
use miru_agent::filesys;
use miru_agent::models::Device;

// external crates
use tokio::task::JoinHandle;

pub async fn create_token_manager(
    dir: &filesys::Dir,
    http_client: Arc<MockClient>,
) -> (TokenManager, JoinHandle<()>) {
    let token_file = TokenFile::new_with_default(dir.file("token.json"), Token::default())
        .await
        .unwrap();
    let private_key_file = dir.file("private_key.pem");
    test_files::seed(&private_key_file, "private_key").await;
    let public_key_file = dir.file("public_key.pem");
    test_files::seed(&public_key_file, "public_key").await;

    TokenManager::spawn(
        32,
        http_client.clone(),
        token_file,
        private_key_file,
        public_key_file,
    )
    .unwrap()
}

pub async fn create_storage(dir: &filesys::Dir) -> Storage {
    let (cfg_inst_stor, _) = CfgInsts::spawn(16, dir.file("cfg_inst_cache.json"), 1000)
        .await
        .unwrap();
    let (cfg_inst_content_stor, _) =
        CfgInstContent::spawn(16, dir.subdir("cfg_inst_content_cache"), 1000)
            .await
            .unwrap();
    let (deployment_stor, _) = Deployments::spawn(16, dir.file("deployment_cache.json"), 1000)
        .await
        .unwrap();
    let (device_stor, _) =
        disk::Device::spawn_with_default(64, dir.file("device.json"), Device::default())
            .await
            .unwrap();
    let (release_stor, _) = Releases::spawn(16, dir.file("releases_cache.json"), 1000)
        .await
        .unwrap();
    let (git_commit_stor, _) = GitCommits::spawn(16, dir.file("git_commits_cache.json"), 1000)
        .await
        .unwrap();
    let (file_rule_stor, _) = FileRules::spawn(16, dir.file("file_rules_cache.json"), 1000)
        .await
        .unwrap();

    Storage {
        device: Arc::new(device_stor),
        cfg_insts: CfgInstStor {
            meta: Arc::new(cfg_inst_stor),
            content: Arc::new(cfg_inst_content_stor),
        },
        deployments: Arc::new(deployment_stor),
        releases: Arc::new(release_stor),
        file_rules: Arc::new(file_rule_stor),
        git_commits: Arc::new(git_commit_stor),
    }
}
