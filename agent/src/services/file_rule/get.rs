// internal crates
use crate::disk;
use crate::models;
use crate::services::{backend::BackendFetcher, errors::ServiceErr};

// external crates
use tracing::error;

pub async fn get<B: BackendFetcher>(
    file_rules: &disk::FileRules,
    backend: &B,
    id: String,
) -> Result<models::FileRule, ServiceErr> {
    if let Some(rule) = file_rules.read_optional(id.clone()).await? {
        return Ok(rule);
    }
    let rule = models::FileRule::from(backend.fetch_file_rule(&id).await?);
    if let Err(e) = file_rules
        .write_if_absent(id.clone(), rule.clone(), |_, _| false)
        .await
    {
        error!("failed to cache file rule {id}: {e}");
    }
    Ok(rule)
}
