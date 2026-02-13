// standard library
use std::fmt;
use std::sync::Arc;

// internal crates
use crate::http::client::HTTPClient;
use crate::http::errors::HTTPErr;
use crate::http::expand::format_expand_query;
use crate::http::pagination::{Pagination, MAX_PAGINATE_LIMIT};
use crate::http::query::build_query_params;
use openapi_client::models::{
    Deployment, DeploymentActivityStatus, DeploymentList, UpdateDeploymentRequest,
};

#[allow(async_fn_in_trait)]
pub trait DeploymentsExt: Send + Sync {
    /// List deployments with optional activity_status filter, expansions, and pagination
    async fn list_deployments<I>(
        &self,
        activity_status_filter: &[DeploymentActivityStatus],
        expansions: I,
        pagination: &Pagination,
        token: &str,
    ) -> Result<DeploymentList, HTTPErr>
    where
        I: IntoIterator + Send,
        I::Item: fmt::Display;

    /// List all deployments by paginating through all pages
    async fn list_all_deployments<I>(
        &self,
        activity_status_filter: &[DeploymentActivityStatus],
        expansions: I,
        token: &str,
    ) -> Result<Vec<Deployment>, HTTPErr>
    where
        I: IntoIterator + Send + Clone,
        I::Item: fmt::Display;

    /// Get a deployment by ID
    async fn get_deployment<I>(
        &self,
        deployment_id: &str,
        expansions: I,
        token: &str,
    ) -> Result<Deployment, HTTPErr>
    where
        I: IntoIterator + Send,
        I::Item: fmt::Display;

    /// Update a deployment
    async fn update_deployment<I>(
        &self,
        deployment_id: &str,
        updates: &UpdateDeploymentRequest,
        expansions: I,
        token: &str,
    ) -> Result<Deployment, HTTPErr>
    where
        I: IntoIterator + Send,
        I::Item: fmt::Display;
}

impl HTTPClient {
    fn deployments_url(&self) -> String {
        format!("{}/deployments", self.base_url)
    }

    fn deployment_url(&self, deployment_id: &str) -> String {
        format!("{}/{}", self.deployments_url(), deployment_id)
    }
}

fn format_activity_status_filter(statuses: &[DeploymentActivityStatus]) -> Option<String> {
    if statuses.is_empty() {
        return None;
    }
    let values: Vec<String> = statuses.iter().map(|s| s.to_string()).collect();
    Some(format!("activity_status={}", values.join(",")))
}

impl DeploymentsExt for HTTPClient {
    async fn list_deployments<I>(
        &self,
        activity_status_filter: &[DeploymentActivityStatus],
        expansions: I,
        pagination: &Pagination,
        token: &str,
    ) -> Result<DeploymentList, HTTPErr>
    where
        I: IntoIterator + Send,
        I::Item: fmt::Display,
    {
        let search_query = format_activity_status_filter(activity_status_filter);
        let expand_query = format_expand_query(expansions);
        let query_params =
            build_query_params(search_query.as_deref(), expand_query.as_deref(), pagination);

        let url = format!("{}{}", self.deployments_url(), query_params);
        let (request, context) = self.build_get_request(&url, self.default_timeout, Some(token))?;
        let response = self.send_cached(url, request, &context).await?.0;
        self.parse_json_response_text::<DeploymentList>(response, &context)
            .await
    }

    async fn list_all_deployments<I>(
        &self,
        activity_status_filter: &[DeploymentActivityStatus],
        expansions: I,
        token: &str,
    ) -> Result<Vec<Deployment>, HTTPErr>
    where
        I: IntoIterator + Send + Clone,
        I::Item: fmt::Display,
    {
        let mut all_deployments = Vec::new();
        let mut pagination = Pagination {
            limit: MAX_PAGINATE_LIMIT,
            offset: 0,
        };

        loop {
            let page = self
                .list_deployments(
                    activity_status_filter,
                    expansions.clone(),
                    &pagination,
                    token,
                )
                .await?;
            all_deployments.extend(page.data);
            if !page.has_more {
                break;
            }
            pagination.offset += pagination.limit;
        }

        Ok(all_deployments)
    }

    async fn get_deployment<I>(
        &self,
        deployment_id: &str,
        expansions: I,
        token: &str,
    ) -> Result<Deployment, HTTPErr>
    where
        I: IntoIterator + Send,
        I::Item: fmt::Display,
    {
        // Build query string with expansions
        let expand_query = format_expand_query(expansions);
        let query_params = if let Some(expand) = expand_query {
            format!("?{}", expand)
        } else {
            String::new()
        };

        // Build the request
        let url = format!("{}{}", self.deployment_url(deployment_id), query_params);
        let (request, context) = self.build_get_request(&url, self.default_timeout, Some(token))?;

        // Send the request (with caching)
        let response = self.send_cached(url, request, &context).await?.0;

        // Parse the response
        self.parse_json_response_text::<Deployment>(response, &context)
            .await
    }

    async fn update_deployment<I>(
        &self,
        deployment_id: &str,
        updates: &UpdateDeploymentRequest,
        expansions: I,
        token: &str,
    ) -> Result<Deployment, HTTPErr>
    where
        I: IntoIterator + Send,
        I::Item: fmt::Display,
    {
        // Build query string with expansions
        let expand_query = format_expand_query(expansions);
        let query_params = if let Some(expand) = expand_query {
            format!("?{}", expand)
        } else {
            String::new()
        };

        // Build the request
        let url = format!("{}{}", self.deployment_url(deployment_id), query_params);
        let (request, context) = self.build_patch_request(
            &url,
            self.marshal_json_payload(updates)?,
            self.default_timeout,
            Some(token),
        )?;

        // Send the request (no caching for updates)
        let http_resp = self.send(request, &context).await?;
        let text_resp = self.handle_response(http_resp, &context).await?;

        // Parse the response
        self.parse_json_response_text::<Deployment>(text_resp, &context)
            .await
    }
}

impl DeploymentsExt for Arc<HTTPClient> {
    async fn list_deployments<I>(
        &self,
        activity_status_filter: &[DeploymentActivityStatus],
        expansions: I,
        pagination: &Pagination,
        token: &str,
    ) -> Result<DeploymentList, HTTPErr>
    where
        I: IntoIterator + Send,
        I::Item: fmt::Display,
    {
        self.as_ref()
            .list_deployments(activity_status_filter, expansions, pagination, token)
            .await
    }

    async fn list_all_deployments<I>(
        &self,
        activity_status_filter: &[DeploymentActivityStatus],
        expansions: I,
        token: &str,
    ) -> Result<Vec<Deployment>, HTTPErr>
    where
        I: IntoIterator + Send + Clone,
        I::Item: fmt::Display,
    {
        self.as_ref()
            .list_all_deployments(activity_status_filter, expansions, token)
            .await
    }

    async fn get_deployment<I>(
        &self,
        deployment_id: &str,
        expansions: I,
        token: &str,
    ) -> Result<Deployment, HTTPErr>
    where
        I: IntoIterator + Send,
        I::Item: fmt::Display,
    {
        self.as_ref()
            .get_deployment(deployment_id, expansions, token)
            .await
    }

    async fn update_deployment<I>(
        &self,
        deployment_id: &str,
        updates: &UpdateDeploymentRequest,
        expansions: I,
        token: &str,
    ) -> Result<Deployment, HTTPErr>
    where
        I: IntoIterator + Send,
        I::Item: fmt::Display,
    {
        self.as_ref()
            .update_deployment(deployment_id, updates, expansions, token)
            .await
    }
}
