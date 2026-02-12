// standard library
use std::fmt;
use std::sync::Arc;

// internal crates
use crate::http::client::HTTPClient;
use crate::http::errors::HTTPErr;
use crate::http::expand::format_expand_query;
use openapi_client::models::{
    Deployment, UpdateDeploymentRequest,
};

#[allow(async_fn_in_trait)]
pub trait DeploymentsExt: Send + Sync {
    /// Get a deployment by ID
    /// 
    /// # Arguments
    /// * `deployment_id` - The ID of the deployment to retrieve
    /// * `expansions` - Optional iterator of expansions (e.g., "release", "config_instances")
    /// * `token` - Authentication token
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
    /// 
    /// # Arguments
    /// * `deployment_id` - The ID of the deployment to update
    /// * `updates` - The update request containing activity_status and/or error_status
    /// * `expansions` - Optional iterator of expansions (e.g., "release", "config_instances")
    /// * `token` - Authentication token
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

impl DeploymentsExt for HTTPClient {
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

