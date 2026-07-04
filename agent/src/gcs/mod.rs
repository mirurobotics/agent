//! Remote Google Cloud Storage (GCS) object storage.
//!
//! This module talks to a GCS bucket over the network. It is the GCS sibling of
//! the S3 object-storage module and, like it, is distinct from [`crate::disk`],
//! which manages local on-disk device state (device.json, settings.json, ...).
//! Do not conflate the two.
//!
//! A [`GcsStore`] is constructed **only** from a caller-supplied short-lived
//! OAuth2 access token plus a bucket name. It never reads ambient Google
//! configuration (Application Default Credentials, environment variables, the
//! GCE/GKE metadata server): the Miru backend mints short-lived tokens and hands
//! them to the agent, mirroring how the S3 module takes short-lived STS
//! credentials.
//!
//! Object bodies are streamed to and from disk (never buffered whole in memory)
//! so the agent can move multi-gigabyte artifacts on memory-constrained devices.
//! Uploads use the resumable `write_object` path (256 KiB chunks, `Seek`-based),
//! and downloads copy the response body chunk-by-chunk.
//!
//! GCS exposes two transports: the object-data client ([`Storage`], HTTP/JSON)
//! handles uploads/downloads, and the control-plane client ([`StorageControl`],
//! gRPC) handles delete and existence checks.
//!
//! A missing object surfaces as [`Code::ResourceNotFound`](crate::errors::Code)
//! (HTTP 404), exactly as in the S3 module.
//!
//! TODO: a real-cloud GCS integration test (mirroring
//! `backend/tests/pkg/gcp/gcs/gcs_test.go`, run against a live bucket via
//! Workload Identity Federation) is a deferred follow-up blocked on infra; see
//! `plans/completed/20260704-gcs-object-storage-crud.md`.

// standard crates
use std::path::Path;

// internal crates
use crate::trace;

// external crates
use google_cloud_auth::credentials::{
    CacheableResource, Credentials, CredentialsProvider, EntityTag,
};
use google_cloud_storage::client::{Storage, StorageControl};
use http::{Extensions, HeaderMap, HeaderValue};
use tokio::io::AsyncWriteExt as _;

pub mod errors;

pub use errors::GcsErr;
use errors::{is_not_found, map_gcs_err, InvalidResponseErr};

/// A [`CredentialsProvider`] that emits a fixed `Authorization: Bearer <token>`
/// header. Built once at [`GcsStore`] construction from a caller-supplied
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

/// An async GCS client scoped to a single bucket, holding the HTTP data client
/// ([`Storage`]) for uploads/downloads and the gRPC control client
/// ([`StorageControl`]) for delete/exists.
pub struct GcsStore {
    data: Storage,
    control: StorageControl,
    /// The bucket resource path, `projects/_/buckets/<bucket>`. GCS clients take
    /// the bucket in this form and the object as the bare key.
    bucket_resource: String,
}

impl GcsStore {
    /// Builds a store from a caller-supplied short-lived OAuth2 access token and
    /// bucket name. The optional `endpoint` overrides the GCS endpoint (used by
    /// tests to point at a local mock server); production callers pass `None`.
    ///
    /// Building performs async client setup but makes no real GCS call, so it is
    /// safe to run in a `#[tokio::test]`. Returns an error if the token cannot be
    /// encoded as an HTTP header value.
    pub async fn new(
        access_token: String,
        bucket: String,
        endpoint: Option<String>,
    ) -> Result<Self, GcsErr> {
        let header_value =
            HeaderValue::from_str(&format!("Bearer {access_token}")).map_err(|e| {
                GcsErr::InvalidResponseErr(InvalidResponseErr {
                    operation: "new".to_string(),
                    msg: format!("access token is not a valid HTTP header value: {e}"),
                    trace: trace!(),
                })
            })?;
        let credentials = Credentials::from(StaticTokenCredentials {
            header_value,
            entity_tag: EntityTag::new(),
        });

        let mut data_builder = Storage::builder().with_credentials(credentials.clone());
        let mut control_builder = StorageControl::builder().with_credentials(credentials);
        if let Some(ep) = endpoint {
            data_builder = data_builder.with_endpoint(ep.clone());
            control_builder = control_builder.with_endpoint(ep);
        }
        let data = data_builder
            .build()
            .await
            .map_err(|e| Self::build_err("data client", e))?;
        let control = control_builder
            .build()
            .await
            .map_err(|e| Self::build_err("control client", e))?;

        Ok(Self {
            data,
            control,
            bucket_resource: format!("projects/_/buckets/{bucket}"),
        })
    }

    /// Test-only constructor that injects a stubbed [`StorageControl`] (for the
    /// gRPC delete/exists ops) while pointing the HTTP data client at `endpoint`.
    /// Mirrors s3's `with_http_client`: the delete/exists tests drive the stub,
    /// and the data client is never called by those two ops.
    #[cfg(feature = "test")]
    pub async fn with_control_stub<T>(
        stub: T,
        bucket: String,
        endpoint: String,
    ) -> Result<Self, GcsErr>
    where
        T: google_cloud_storage::stub::StorageControl + 'static,
    {
        let credentials = Credentials::from(StaticTokenCredentials {
            header_value: HeaderValue::from_static("Bearer test-token"),
            entity_tag: EntityTag::new(),
        });
        let data = Storage::builder()
            .with_credentials(credentials)
            .with_endpoint(endpoint)
            .build()
            .await
            .map_err(|e| Self::build_err("data client", e))?;
        Ok(Self {
            data,
            control: StorageControl::from_stub(stub),
            bucket_resource: format!("projects/_/buckets/{bucket}"),
        })
    }

    /// Creates or overwrites an object by streaming a file off disk. The whole
    /// file is never held in memory: the resumable `write_object` path reads it
    /// in bounded chunks with `Seek` support.
    pub async fn put_object(&self, key: &str, path: &Path) -> Result<(), GcsErr> {
        let file = tokio::fs::File::open(path)
            .await
            .map_err(|e| self.map_body_io_err("put_object", key, path, e))?;
        self.data
            .write_object(&self.bucket_resource, key, file)
            .send_unbuffered()
            .await
            .map_err(|e| map_gcs_err("put_object", Some(key), e))?;
        Ok(())
    }

    /// Streams an object's body to a destination file, chunk-by-chunk. A missing
    /// object maps to [`GcsErr::ObjectNotFoundErr`].
    pub async fn get_object(&self, key: &str, dest: &Path) -> Result<(), GcsErr> {
        let mut resp = self
            .data
            .read_object(&self.bucket_resource, key)
            .send()
            .await
            .map_err(|e| self.classify_read_err(key, e))?;

        let mut file = tokio::fs::File::create(dest)
            .await
            .map_err(|e| self.map_body_io_err("get_object", key, dest, e))?;
        while let Some(chunk) = resp.next().await {
            let bytes = chunk.map_err(|e| map_gcs_err("get_object", Some(key), e))?;
            file.write_all(&bytes)
                .await
                .map_err(|e| self.map_body_io_err("get_object", key, dest, e))?;
        }
        file.flush()
            .await
            .map_err(|e| self.map_body_io_err("get_object", key, dest, e))?;
        Ok(())
    }

    /// Deletes an object. Idempotent: a `NOT_FOUND` from the control plane (the
    /// object was already gone) is treated as success, mirroring s3's contract.
    pub async fn delete_object(&self, key: &str) -> Result<(), GcsErr> {
        match self
            .control
            .delete_object()
            .set_bucket(&self.bucket_resource)
            .set_object(key)
            .send()
            .await
        {
            Ok(()) => Ok(()),
            Err(e) if is_not_found(&e) => Ok(()),
            Err(e) => Err(map_gcs_err("delete_object", Some(key), e)),
        }
    }

    /// Returns `true` if the object exists, `false` on a `NOT_FOUND`. Other
    /// errors propagate.
    pub async fn object_exists(&self, key: &str) -> Result<bool, GcsErr> {
        match self
            .control
            .get_object()
            .set_bucket(&self.bucket_resource)
            .set_object(key)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(e) if is_not_found(&e) => Ok(false),
            Err(e) => Err(map_gcs_err("get_object", Some(key), e)),
        }
    }

    /// Classifies a `read_object` send error: a not-found becomes
    /// [`GcsErr::ObjectNotFoundErr`]; everything else delegates to the common
    /// mapper.
    fn classify_read_err(&self, key: &str, err: google_cloud_gax::error::Error) -> GcsErr {
        if is_not_found(&err) {
            GcsErr::ObjectNotFoundErr(errors::ObjectNotFoundErr {
                key: key.to_string(),
                trace: trace!(),
            })
        } else {
            map_gcs_err("get_object", Some(key), err)
        }
    }

    /// Maps a filesystem I/O error hit while streaming an object body (opening
    /// the source, creating the destination, or copying) into a `GcsErr`.
    fn map_body_io_err(
        &self,
        operation: &str,
        key: &str,
        path: &Path,
        err: std::io::Error,
    ) -> GcsErr {
        GcsErr::InvalidResponseErr(InvalidResponseErr {
            operation: operation.to_string(),
            msg: format!(
                "filesystem I/O error for object '{key}' at path '{}': {err}",
                path.display()
            ),
            trace: trace!(),
        })
    }

    /// Maps a client-builder error into a `GcsErr`.
    fn build_err(which: &str, err: google_cloud_gax::client_builder::Error) -> GcsErr {
        GcsErr::InvalidResponseErr(InvalidResponseErr {
            operation: "new".to_string(),
            msg: format!("failed to build GCS {which}: {err}"),
            trace: trace!(),
        })
    }
}
