// internal crates
use crate::http::{errors::HTTPErr, path, query::QueryParams, request, ClientI};
use backend_api::models::BaseFileRule;

// ================================ FREE FUNCTIONS ================================= //

pub async fn get(
    client: &impl ClientI,
    id: &str,
    expansions: &[&str],
    token: &str,
) -> Result<BaseFileRule, HTTPErr> {
    let qp = QueryParams::new().expand(expansions);
    let url = path::url(client.base_url(), &["file_rules", id])?;
    let request = request::Params::get(&url).with_query(qp).with_token(token);
    super::client::fetch(client, request).await
}
