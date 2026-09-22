// internal crates
use crate::disk;
use crate::models;
use crate::services::errors::ServiceErr;

pub async fn get(file_rules: &disk::FileRules, id: String) -> Result<models::FileRule, ServiceErr> {
    Ok(file_rules.read(id).await?)
}
