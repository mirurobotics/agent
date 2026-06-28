// internal crates
use crate::http::{
    errors::HTTPErr,
    query::{Page, QueryParams, MAX_PAGE_LIMIT},
    request, ClientI,
};
use backend_api::models::{BaseUploadRule, UploadRuleList};

// ================================ PARAM STRUCTS ================================== //

pub struct ListParams<'a> {
    pub pagination: &'a Page,
    pub token: &'a str,
}

pub struct ListAllParams<'a> {
    pub token: &'a str,
}

// ================================ FREE FUNCTIONS ================================= //

pub async fn list(
    client: &impl ClientI,
    params: ListParams<'_>,
) -> Result<UploadRuleList, HTTPErr> {
    let qp = QueryParams::new().paginate(params.pagination);

    let url = format!("{}/upload_rules", client.base_url());
    let request = request::Params::get(&url)
        .with_query(qp)
        .with_token(params.token);
    super::client::fetch(client, request).await
}

pub async fn list_all(
    client: &impl ClientI,
    params: ListAllParams<'_>,
) -> Result<Vec<BaseUploadRule>, HTTPErr> {
    let mut all_upload_rules = Vec::new();
    let mut pagination = Page {
        limit: MAX_PAGE_LIMIT,
        offset: 0,
    };

    loop {
        let page = list(
            client,
            ListParams {
                pagination: &pagination,
                token: params.token,
            },
        )
        .await?;
        all_upload_rules.extend(page.data);
        if !page.has_more {
            break;
        }
        pagination.offset += pagination.limit;
    }

    Ok(all_upload_rules)
}
