//! Remote S3 object storage.
//!
//! This module talks to an S3 bucket over the network. It is distinct from
//! [`crate::disk`], which manages local on-disk device state (device.json,
//! settings.json, ...). Do not conflate the two.
//!
//! A [`Store`] is constructed **only** from caller-supplied temporary
//! credentials (access key id, secret access key, session token) plus a region.
//! It is bucket-agnostic: each operation targets an [`Object`] carrying its own
//! bucket and key. It never reads ambient AWS configuration (environment
//! variables, `~/.aws`, or EC2/ECS instance metadata): the Miru backend mints
//! short-lived STS credentials and hands them to the agent.
//!
//! Object bodies are streamed to and from disk (never buffered whole in
//! memory) so the agent can move multi-gigabyte artifacts on memory-constrained
//! devices.
//!
//! [`Store`] is a thin, stateless CRUD client: [`Store::put`],
//! [`Store::get`], [`Store::delete`], and [`Store::exists`].

// internal crates
use crate::filesys::file::File;
use crate::filesys::path::PathExt;
use crate::trace;

// external crates
use aws_sdk_s3::config::{BehaviorVersion, Credentials as AwsCredentials, Region};
use aws_sdk_s3::operation::get_object::GetObjectError;
use aws_sdk_s3::primitives::{ByteStream, ByteStreamError};
use aws_sdk_s3::Client;

pub mod errors;

pub use errors::S3Err;
use errors::{InvalidResponseErr, ObjectNotFoundErr};

pub struct Config {
    pub creds: Credentials,
    pub region: String,
}

pub struct Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: String,
}

impl Default for Credentials {
    fn default() -> Self {
        Self {
            access_key_id: "access-key".to_string(),
            secret_access_key: "secret-key".to_string(),
            session_token: "session-token".to_string(),
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
        write!(f, "s3://{}/{}", self.bucket, self.key)
    }
}

pub struct Store {
    client: Client,
}

impl Store {
    /// Builds a client from caller-supplied temporary credentials. No network
    /// I/O happens here; the first request is made lazily on the first call.
    pub fn new(cfg: Config) -> Self {
        let s3creds = AwsCredentials::new(
            cfg.creds.access_key_id,
            cfg.creds.secret_access_key,
            Some(cfg.creds.session_token),
            None,
            "miru-agent",
        );
        let s3cfg = aws_sdk_s3::config::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(cfg.region))
            .credentials_provider(s3creds)
            .build();
        Self {
            client: Client::from_conf(s3cfg),
        }
    }

    /// Test-only constructor that injects a caller-provided HTTP client (e.g. a
    /// `StaticReplayClient`) in place of the default HTTPS connector, so tests
    /// serve canned responses without touching the network. Credentials are
    /// static dummies since no real signing endpoint is contacted.
    #[cfg(feature = "test")]
    pub fn from_http_client(
        http_client: impl aws_sdk_s3::config::HttpClient + 'static,
        cfg: Config,
    ) -> Self {
        let s3creds = AwsCredentials::new("test-access-key", "test-secret-key", None, None, "test");
        let s3cfg = aws_sdk_s3::config::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(cfg.region))
            .credentials_provider(s3creds)
            // Path-style URLs (`/<bucket>/<key>`) make the replayed request URIs
            // deterministic and readable, so tests can assert on the exact path.
            .force_path_style(true)
            .http_client(http_client)
            .build();
        Self {
            client: Client::from_conf(s3cfg),
        }
    }

    /// Creates or overwrites an object by streaming a file off disk.
    ///
    /// This CRUD client streams the whole file through a single `PutObject`
    /// (see [`Self::put_singlepart`]). Size-based routing to a multipart upload
    /// for large files arrives with the multipart module in a follow-up PR.
    pub async fn put(&self, src: File, dst: &Object) -> Result<(), S3Err> {
        self.put_singlepart(&src, dst).await
    }

    /// Streams a file to S3 as a single-part upload.
    pub async fn put_singlepart(&self, src: &File, dst: &Object) -> Result<(), S3Err> {
        let body = ByteStream::from_path(src.path())
            .await
            .map_err(|e| self.map_bytestream_err("put_object", dst, src, &e))?;
        self.client
            .put_object()
            .bucket(&dst.bucket)
            .key(&dst.key)
            .body(body)
            .send()
            .await
            .map_err(|e| errors::map_sdk_err_common("put_object", Some(dst.key.to_string()), e))?;
        Ok(())
    }

    /// Streams an object's body to a destination file. A missing object maps to
    /// [`S3Err::ObjectNotFoundErr`]. The body is copied through a bounded buffer rather
    /// than collected into memory.
    pub async fn get(&self, src: &Object, dest: &File) -> Result<(), S3Err> {
        let output = match self
            .client
            .get_object()
            .bucket(&src.bucket)
            .key(&src.key)
            .send()
            .await
        {
            Ok(output) => output,
            Err(err) => {
                let is_not_found = errors::is_not_found(&err)
                    || matches!(err.as_service_error(), Some(GetObjectError::NoSuchKey(_)));
                if is_not_found {
                    return Err(S3Err::ObjectNotFoundErr(ObjectNotFoundErr {
                        key: src.key.to_string(),
                        trace: trace!(),
                    }));
                }
                return Err(errors::map_sdk_err_common(
                    "get_object",
                    Some(src.key.to_string()),
                    err,
                ));
            }
        };

        let mut reader = output.body.into_async_read();
        let mut file = tokio::fs::File::create(dest.path())
            .await
            .map_err(|e| self.map_body_io_err("get_object", src, dest, e))?;
        tokio::io::copy(&mut reader, &mut file)
            .await
            .map_err(|e| self.map_body_io_err("get_object", src, dest, e))?;
        Ok(())
    }

    /// Deletes an object. Idempotent per S3 semantics (deleting a missing key still
    /// returns success).
    pub async fn delete(&self, obj: &Object) -> Result<(), S3Err> {
        self.client
            .delete_object()
            .bucket(&obj.bucket)
            .key(&obj.key)
            .send()
            .await
            .map_err(|e| {
                errors::map_sdk_err_common("delete_object", Some(obj.key.to_string()), e)
            })?;
        Ok(())
    }

    /// Returns `true` if the object exists (HEAD 200), `false` on a 404. Other errors
    /// propagate.
    pub async fn exists(&self, obj: &Object) -> Result<bool, S3Err> {
        match self
            .client
            .head_object()
            .bucket(&obj.bucket)
            .key(&obj.key)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(err) => {
                if errors::is_not_found(&err) {
                    Ok(false)
                } else {
                    Err(errors::map_sdk_err_common(
                        "head_object",
                        Some(obj.key.to_string()),
                        err,
                    ))
                }
            }
        }
    }

    /// Maps a filesystem I/O error hit while streaming an object body (opening the
    /// source, creating the destination, or copying) into an `S3Err`.
    fn map_body_io_err(
        &self,
        operation: &str,
        obj: &Object,
        file: &File,
        err: std::io::Error,
    ) -> S3Err {
        S3Err::InvalidResponseErr(InvalidResponseErr {
            operation: operation.to_string(),
            msg: format!("filesystem I/O error for object '{obj}' at path '{file}': {err}"),
            trace: trace!(),
        })
    }

    /// Maps a [`ByteStream`] construction error (e.g. the file could not be opened or
    /// read) into an `S3Err`.
    fn map_bytestream_err(
        &self,
        operation: &str,
        obj: &Object,
        file: &File,
        err: &ByteStreamError,
    ) -> S3Err {
        S3Err::InvalidResponseErr(InvalidResponseErr {
            operation: operation.to_string(),
            msg: format!("failed to open '{file}' for streaming object '{obj}': {err}"),
            trace: trace!(),
        })
    }
}
