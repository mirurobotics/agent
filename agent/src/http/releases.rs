// internal crates
use crate::http::{errors::HTTPErr, query::QueryParams, request, ClientI};
use backend_api::models::Release;

// ================================ FREE FUNCTIONS ================================= //

pub async fn get(
    client: &impl ClientI,
    id: &str,
    expansions: &[&str],
    token: &str,
) -> Result<Release, HTTPErr> {
    let qp = QueryParams::new().expand(expansions);
    let url = format!("{}/releases/{}", client.base_url(), id);
    let request = request::Params::get(&url).with_query(qp).with_token(token);
    super::client::fetch(client, request).await
}
