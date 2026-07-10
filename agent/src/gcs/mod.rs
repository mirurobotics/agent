//! Remote Google Cloud Storage (GCS) object storage.
//!
//! This module talks to a GCS bucket over the network. Its public API mirrors
//! the S3 object-storage module: a [`Store`] exposes [`Store::put`],
//! [`Store::put_singlepart`], [`Store::get`], [`Store::delete`], and
//! [`Store::exists`] over the [`Credentials`], [`Object`], and
//! [`crate::filesys::file::File`] value types, so `gcs::Store` reads as a
//! faithful sibling of `s3::Store`. Like the S3 module it is distinct from
//! [`crate::disk`], which manages local on-disk device state (device.json,
//! settings.json, ...). Do not conflate the two.
//!
//! A [`Store`] is constructed **only** from a caller-supplied short-lived
//! OAuth2 access token. It never reads ambient Google configuration
//! (Application Default Credentials, environment variables, the GCE/GKE
//! metadata server): the Miru backend mints short-lived tokens and hands them
//! to the agent, mirroring how the S3 module takes short-lived STS credentials.
//! The target bucket is not held on the `Store`; it rides on each [`Object`],
//! mirroring s3 where the bucket travels with the `Object`.
//!
//! Object bodies are streamed to and from disk (never buffered whole in memory)
//! so the agent can move multi-gigabyte artifacts on memory-constrained devices.
//! Uploads use the resumable `write_object` path (256 KiB chunks, `Seek`-based),
//! and downloads copy the response body chunk-by-chunk.
//!
//! GCS exposes two transports: the object-data client ([`Storage`], HTTP/JSON)
//! handles uploads/downloads, and the control-plane client ([`StorageControl`],
//! gRPC) handles delete and existence checks. s3 has a single HTTP client; this
//! is a genuine GCS-forced deviation that also shapes the test seams below.
//!
//! A missing object surfaces as [`Code::ResourceNotFound`](crate::errors::Code)
//! (HTTP 404), exactly as in the S3 module.
//!
//! TODO: a real-cloud GCS integration test (mirroring
//! `backend/tests/pkg/gcp/gcs/gcs_test.go`, run against a live bucket via
//! Workload Identity Federation) is a deferred follow-up blocked on infra; see
//! `plans/completed/20260704-gcs-object-storage-crud.md`.

// internal crates
use crate::filesys::{file::File, files, path::PathExt};
use crate::trace;

// external crates
use google_cloud_auth::credentials::{
    CacheableResource, Credentials as GcsCredentials, CredentialsProvider, EntityTag,
};
use google_cloud_storage::client::{Storage, StorageControl};
use http::{Extensions, HeaderMap, HeaderValue};
use tokio::io::AsyncWriteExt as _;

pub mod errors;

pub use errors::GcsErr;
use errors::{is_not_found, map_gcs_err, InvalidResponseErr};

/// Caller-facing credentials: the short-lived OAuth2 access token the backend
/// mints. Internally this is turned into the [`StaticTokenCredentials`] provider
/// the GCS SDK plumbs through; `Credentials` is the value type callers see,
/// mirroring `s3::Credentials`.
pub struct Credentials {
    pub access_token: String,
}

#[cfg(feature = "test")]
impl Default for Credentials {
    fn default() -> Self {
        Self {
            access_token: "test-token".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    pub bucket: String,
    pub key: String,
}

impl std::fmt::Display for Object {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // GCS's canonical URI scheme is `gs://`, the one honest deviation from
        // s3's `s3://` prefix.
        write!(f, "gs://{}/{}", self.bucket, self.key)
    }
}

impl Object {
    /// The GCS resource name of this object's bucket
    /// (`projects/_/buckets/<bucket>`). Both transports address a bucket by this
    /// path, so an `Object` fully identifies its target — mirroring how s3 takes
    /// the bucket straight from the `Object`. This wire-path assembly has no s3
    /// analog: it is a GCS-forced detail, kept in one place instead of rebuilt at
    /// each call site.
    fn bucket_resource(&self) -> String {
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

/// An async GCS client holding the HTTP data client ([`Storage`]) for
/// uploads/downloads and the gRPC control client ([`StorageControl`]) for
/// delete/exists.
pub struct Store {
    data: Storage,
    control: StorageControl,
}

impl Store {
    /// Builds a store from caller-supplied credentials (a short-lived OAuth2
    /// access token).
    ///
    /// Unlike s3's `new`, this is fallible and async: fallible because the token
    /// is turned into an `Authorization` header value and an invalid byte in the
    /// token makes that conversion fail; async because the GCS client builders
    /// are async. Building performs async client setup but makes no real GCS
    /// call, so it is safe to run in a `#[tokio::test]`.
    pub async fn new(creds: Credentials) -> Result<Self, GcsErr> {
        Self::build(creds, None, None).await
    }

    /// Test-only seam pointing the HTTP data client at `endpoint` (a local mock
    /// server) while building a real gRPC control client. Used by the put/get
    /// data-path tests; the control client is never called by those ops.
    #[cfg(feature = "test")]
    pub async fn from_endpoint(creds: Credentials, endpoint: String) -> Result<Self, GcsErr> {
        Self::build(creds, Some(endpoint), None).await
    }

    /// Test-only seam that injects a stubbed [`StorageControl`] for the gRPC
    /// delete/exists ops while pointing the HTTP data client at `endpoint`.
    ///
    /// s3 injects one HTTP client via `from_http_client`; GCS must inject a gRPC
    /// control stub AND point the HTTP data client at `endpoint`, so this seam
    /// takes both — the closest the dual-transport reality gets to s3's single
    /// `(http_client, credentials)` pairing.
    #[cfg(feature = "test")]
    pub async fn from_stub<T>(stub: T, creds: Credentials, endpoint: String) -> Result<Self, GcsErr>
    where
        T: google_cloud_storage::stub::StorageControl + 'static,
    {
        Self::build(creds, Some(endpoint), Some(StorageControl::from_stub(stub))).await
    }

    /// Shared constructor body: builds the `Bearer` header from the token and
    /// constructs both clients (optionally endpoint-overridden for tests).
    /// `control_override` lets tests inject a stubbed control client instead of
    /// building a real one. The target bucket rides on each [`Object`], so it is
    /// not stored on the `Store`.
    async fn build(
        creds: Credentials,
        endpoint: Option<String>,
        control_override: Option<StorageControl>,
    ) -> Result<Self, GcsErr> {
        // Token -> header can fail on an invalid byte, which is why `new` is
        // fallible where s3's is not.
        let header_value = HeaderValue::from_str(&format!("Bearer {}", creds.access_token))
            .map_err(|e| {
                GcsErr::InvalidResponseErr(InvalidResponseErr {
                    operation: "new".to_string(),
                    msg: format!("access token is not a valid HTTP header value: {e}"),
                    trace: trace!(),
                })
            })?;
        let credentials = GcsCredentials::from(StaticTokenCredentials {
            header_value,
            entity_tag: EntityTag::new(),
        });

        let mut data_builder = Storage::builder().with_credentials(credentials.clone());
        if let Some(ep) = endpoint.clone() {
            data_builder = data_builder.with_endpoint(ep);
        }
        let data = data_builder
            .build()
            .await
            .map_err(|e| Self::build_err("data client", e))?;

        let control = match control_override {
            Some(control) => control,
            None => {
                let mut control_builder = StorageControl::builder().with_credentials(credentials);
                if let Some(ep) = endpoint {
                    control_builder = control_builder.with_endpoint(ep);
                }
                control_builder
                    .build()
                    .await
                    .map_err(|e| Self::build_err("control client", e))?
            }
        };

        Ok(Self { data, control })
    }

    /// Creates or overwrites an object by streaming a file off disk.
    ///
    /// The whole file is never held in memory: the `write_object` path reads it
    /// in bounded chunks. `put` reads the source size up front so a missing
    /// local source surfaces as a [`GcsErr::FileSysErr`] exactly like s3's `put`.
    /// Unlike s3 there is no size-routed second path: GCS's `write_object` folds
    /// the multipart/resumable decision inside the SDK, so `put` delegates
    /// straight to [`Self::put_singlepart`] after the size read.
    pub async fn put(&self, src: File, dst: &Object) -> Result<(), GcsErr> {
        let _size = files::size(&src).await?;
        self.put_singlepart(&src, dst).await
    }

    /// Streams a file to GCS via a single `write_object` call. The SDK internally
    /// chooses simple vs resumable transfer by payload size.
    pub async fn put_singlepart(&self, src: &File, dst: &Object) -> Result<(), GcsErr> {
        let resource = dst.bucket_resource();
        let file = tokio::fs::File::open(src.path())
            .await
            .map_err(|e| errors::map_body_io_err("put_object", dst, src, e))?;
        self.data
            .write_object(&resource, &dst.key, file)
            .send_unbuffered()
            .await
            .map_err(|e| map_gcs_err("put_object", Some(&dst.key), e))?;
        Ok(())
    }

    /// Streams an object's body to a destination file, chunk-by-chunk. A missing
    /// object maps to [`GcsErr::ObjectNotFoundErr`].
    pub async fn get(&self, src: &Object, dest: &File) -> Result<(), GcsErr> {
        let resource = src.bucket_resource();
        let mut resp = match self.data.read_object(&resource, &src.key).send().await {
            Ok(resp) => resp,
            Err(e) if is_not_found(&e) => {
                return Err(GcsErr::ObjectNotFoundErr(errors::ObjectNotFoundErr {
                    key: src.key.to_string(),
                    trace: trace!(),
                }))
            }
            Err(e) => return Err(map_gcs_err("get_object", Some(&src.key), e)),
        };

        let mut file = tokio::fs::File::create(dest.path())
            .await
            .map_err(|e| errors::map_body_io_err("get_object", src, dest, e))?;
        while let Some(chunk) = resp.next().await {
            // A wire read error is a `google_cloud_gax::error::Error`, mapped by
            // `map_gcs_err` (GCS has no `ByteStream`, so no separate body-read mapper).
            let bytes = chunk.map_err(|e| map_gcs_err("get_object", Some(&src.key), e))?;
            file.write_all(&bytes)
                .await
                .map_err(|e| errors::map_body_io_err("get_object", src, dest, e))?;
        }
        file.flush()
            .await
            .map_err(|e| errors::map_body_io_err("get_object", src, dest, e))?;
        Ok(())
    }

    /// Deletes an object. Idempotent: a `NOT_FOUND` from the control plane (the
    /// object was already gone) is treated as success, matching s3's idempotent
    /// delete.
    pub async fn delete(&self, obj: &Object) -> Result<(), GcsErr> {
        let resource = obj.bucket_resource();
        match self
            .control
            .delete_object()
            .set_bucket(&resource)
            .set_object(&obj.key)
            .send()
            .await
        {
            Ok(()) => Ok(()),
            Err(e) if is_not_found(&e) => Ok(()),
            Err(e) => Err(map_gcs_err("delete_object", Some(&obj.key), e)),
        }
    }

    /// Returns `true` if the object exists, `false` on a `NOT_FOUND`. Other
    /// errors propagate.
    pub async fn exists(&self, obj: &Object) -> Result<bool, GcsErr> {
        let resource = obj.bucket_resource();
        match self
            .control
            .get_object()
            .set_bucket(&resource)
            .set_object(&obj.key)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(e) if is_not_found(&e) => Ok(false),
            Err(e) => Err(map_gcs_err("get_object", Some(&obj.key), e)),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn provider() -> StaticTokenCredentials {
        StaticTokenCredentials {
            header_value: HeaderValue::from_static("Bearer abc123"),
            entity_tag: EntityTag::new(),
        }
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
