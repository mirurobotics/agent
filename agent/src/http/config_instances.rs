// internal crates
use crate::http::{errors::HTTPErr, path, request, ClientI};

pub struct GetContentParams<'a> {
    pub id: &'a str,
    pub token: &'a str,
}

pub async fn get_content(
    client: &impl ClientI,
    params: GetContentParams<'_>,
) -> Result<String, HTTPErr> {
    let url = path::url(
        client.base_url(),
        &["config_instances", params.id, "content"],
    )?;
    let request = request::Params::get(&url).with_token(params.token);
    let (text, _meta) = client.execute(request).await?;
    Ok(text)
}
