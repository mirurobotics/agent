// external crates
use google_cloud_auth::credentials::{CacheableResource, CredentialsProvider, EntityTag};
use http::{Extensions, HeaderMap, HeaderValue};

pub mod errors;
pub mod store;

pub use errors::GcsErr;
pub use store::Store;

pub struct Credentials {
    pub access_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    pub bucket: String,
    pub key: String,
}

impl std::fmt::Display for Object {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // GCS's canonical URI scheme is `gs://`.
        write!(f, "gs://{}/{}", self.bucket, self.key)
    }
}

impl Object {
    /// The GCS resource name of this object's bucket (`projects/_/buckets/<bucket>`).
    fn resource_name(&self) -> String {
        format!("projects/_/buckets/{}", self.bucket)
    }
}

/// A [`CredentialsProvider`] that emits a fixed `Authorization: Bearer <token>`
/// header. Built once at [`Store`] construction from a caller-supplied
/// short-lived access token; the SDK re-invokes [`Self::headers`] per request.
#[derive(Debug)]
struct StaticTokenCredentials {
    /// Pre-built `Bearer <token>` header value.
    header_value: HeaderValue,
    entity_tag: EntityTag,
}

impl CredentialsProvider for StaticTokenCredentials {
    async fn headers(
        &self,
        extensions: Extensions,
    ) -> std::result::Result<
        CacheableResource<HeaderMap>,
        google_cloud_auth::errors::CredentialsError,
    > {
        match extensions.get::<EntityTag>() {
            Some(tag) if self.entity_tag.eq(tag) => Ok(CacheableResource::NotModified),
            _ => {
                let mut headers = HeaderMap::new();
                headers.insert(http::header::AUTHORIZATION, self.header_value.clone());
                Ok(CacheableResource::New {
                    data: headers,
                    entity_tag: self.entity_tag.clone(),
                })
            }
        }
    }

    async fn universe_domain(&self) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider() -> StaticTokenCredentials {
        let mut header_value = HeaderValue::from_static("Bearer abc123");
        header_value.set_sensitive(true);
        StaticTokenCredentials {
            header_value,
            entity_tag: EntityTag::new(),
        }
    }

    /// The token never appears in Debug output (the SDK requires Debug on
    /// credential providers, and request headers reach trace-level logs).
    #[test]
    fn debug_output_redacts_token() {
        let debug = format!("{:?}", provider());
        assert!(!debug.contains("abc123"), "token leaked: {debug}");
    }

    /// Fresh extensions (no cached `EntityTag`) yield a `New` result carrying an
    /// `Authorization: Bearer <token>` header.
    #[tokio::test]
    async fn headers_emits_bearer_on_new() {
        let provider = provider();
        let result = provider.headers(Extensions::new()).await.unwrap();
        match result {
            CacheableResource::New { data, .. } => {
                let auth = data.get(http::header::AUTHORIZATION).unwrap();
                assert_eq!(auth.to_str().unwrap(), "Bearer abc123");
            }
            CacheableResource::NotModified => panic!("expected New headers"),
        }
    }

    /// Re-presenting the provider's own `EntityTag` yields `NotModified`.
    #[tokio::test]
    async fn headers_returns_not_modified_for_matching_tag() {
        let provider = provider();
        // Extract the tag from the first (New) response, then present it back.
        let CacheableResource::New { entity_tag, .. } =
            provider.headers(Extensions::new()).await.unwrap()
        else {
            panic!("expected New headers");
        };
        let mut extensions = Extensions::new();
        extensions.insert(entity_tag);
        let result = provider.headers(extensions).await.unwrap();
        assert!(matches!(result, CacheableResource::NotModified));
    }

    #[tokio::test]
    async fn universe_domain_is_none() {
        assert!(provider().universe_domain().await.is_none());
    }
}
