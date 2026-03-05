// internal crates
use super::errors::HTTPErr;
use super::request;
use super::ClientI;

pub struct GetContentParams<'a> {
    pub id: &'a str,
    pub token: &'a str,
}

pub async fn get_content(
    client: &impl ClientI,
    params: GetContentParams<'_>,
) -> Result<String, HTTPErr> {
    let url = format!(
        "{}/config_instances/{}/content",
        client.base_url(),
        params.id
    );
    let request = request::Params::get(&url).with_token(params.token);
    let (text, _meta) = client.execute(request).await?;
    Ok(text)
}
