//! Remote S3 object storage.
//!
//! This module talks to an S3 bucket over the network. It is distinct from
//! [`crate::storage`], which manages local on-disk device state (device.json,
//! settings.json, ...). Do not conflate the two.
//!
//! An [`S3Store`] is constructed **only** from caller-supplied temporary
//! credentials (access key id, secret access key, session token) plus a region
//! and bucket name. It never reads ambient AWS configuration (environment
//! variables, `~/.aws`, or EC2/ECS instance metadata): the Miru backend mints
//! short-lived STS credentials and hands them to the agent.

// external crates
use aws_sdk_s3::config::{BehaviorVersion, Credentials as AwsCredentials, Region};
use aws_sdk_s3::operation::get_object::GetObjectError;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client;

pub mod errors;

pub use errors::ObjectStoreErr;
use errors::{InvalidResponseErr, ObjectNotFoundErr};

/// Caller-supplied temporary AWS credentials (from an STS AssumeRole).
pub struct Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: String,
}

/// An async S3 client scoped to a single bucket.
pub struct S3Store {
    client: Client,
    bucket: String,
}

impl S3Store {
    /// Builds a client from caller-supplied temporary credentials. No network
    /// I/O happens here; the first request is made lazily on the first call.
    pub fn new(creds: Credentials, region: String, bucket: String) -> Self {
        let credentials = AwsCredentials::new(
            creds.access_key_id,
            creds.secret_access_key,
            Some(creds.session_token),
            None,
            "miru-agent",
        );
        let config = aws_sdk_s3::config::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(region))
            .credentials_provider(credentials)
            .build();
        Self {
            client: Client::from_conf(config),
            bucket,
        }
    }

    /// Test-only constructor that injects a caller-provided HTTP client (e.g. a
    /// `StaticReplayClient`) in place of the default HTTPS connector, so tests
    /// serve canned responses without touching the network. Credentials are
    /// static dummies since no real signing endpoint is contacted.
    #[cfg(feature = "test")]
    pub fn with_http_client(
        http_client: impl aws_sdk_s3::config::HttpClient + 'static,
        region: String,
        bucket: String,
    ) -> Self {
        let credentials =
            AwsCredentials::new("test-access-key", "test-secret-key", None, None, "test");
        let config = aws_sdk_s3::config::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(region))
            .credentials_provider(credentials)
            .http_client(http_client)
            .build();
        Self {
            client: Client::from_conf(config),
            bucket,
        }
    }

    /// Creates or overwrites an object from an in-memory buffer.
    pub async fn put_object(&self, key: &str, bytes: Vec<u8>) -> Result<(), ObjectStoreErr> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(ByteStream::from(bytes))
            .send()
            .await
            .map_err(|e| errors::map_sdk_err_common("put_object", Some(key.to_string()), e))?;
        Ok(())
    }

    /// Reads an object's whole body into memory. A missing object maps to
    /// [`ObjectStoreErr::ObjectNotFoundErr`].
    pub async fn get_object(&self, key: &str) -> Result<Vec<u8>, ObjectStoreErr> {
        let output = match self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(output) => output,
            Err(err) => {
                let is_not_found = err
                    .raw_response()
                    .map(|r| r.status().as_u16() == 404)
                    .unwrap_or(false)
                    || matches!(err.as_service_error(), Some(GetObjectError::NoSuchKey(_)));
                if is_not_found {
                    return Err(ObjectStoreErr::ObjectNotFoundErr(ObjectNotFoundErr {
                        key: key.to_string(),
                        trace: crate::trace!(),
                    }));
                }
                return Err(errors::map_sdk_err_common(
                    "get_object",
                    Some(key.to_string()),
                    err,
                ));
            }
        };

        let bytes = output.body.collect().await.map_err(|e| {
            ObjectStoreErr::InvalidResponseErr(InvalidResponseErr {
                operation: "get_object".to_string(),
                msg: format!("failed to collect response body: {e}"),
                trace: crate::trace!(),
            })
        })?;
        Ok(bytes.into_bytes().to_vec())
    }

    /// Deletes an object. Idempotent per S3 semantics (deleting a missing key
    /// still returns success).
    pub async fn delete_object(&self, key: &str) -> Result<(), ObjectStoreErr> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| errors::map_sdk_err_common("delete_object", Some(key.to_string()), e))?;
        Ok(())
    }

    /// Returns `true` if the object exists (HEAD 200), `false` on a 404. Other
    /// errors propagate.
    pub async fn object_exists(&self, key: &str) -> Result<bool, ObjectStoreErr> {
        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(err) => {
                let is_not_found = err
                    .raw_response()
                    .map(|r| r.status().as_u16() == 404)
                    .unwrap_or(false);
                if is_not_found {
                    Ok(false)
                } else {
                    Err(errors::map_sdk_err_common(
                        "head_object",
                        Some(key.to_string()),
                        err,
                    ))
                }
            }
        }
    }

    /// Lists object keys under a prefix. Returns a single page; a truncated
    /// response is recorded via `tracing::warn!` (pagination is a follow-up).
    pub async fn list_objects(&self, prefix: &str) -> Result<Vec<String>, ObjectStoreErr> {
        let output = self
            .client
            .list_objects_v2()
            .bucket(&self.bucket)
            .prefix(prefix)
            .send()
            .await
            .map_err(|e| errors::map_sdk_err_common("list_objects_v2", None, e))?;

        if output.is_truncated().unwrap_or(false) {
            tracing::warn!(
                "list_objects response for prefix '{prefix}' was truncated; \
                 only the first page of keys is returned (pagination is a follow-up)"
            );
        }

        let keys = output
            .contents()
            .iter()
            .filter_map(|obj| obj.key().map(|k| k.to_string()))
            .collect();
        Ok(keys)
    }
}
