// internal crates
use crate::filesys::{file::File, files, path::PathExt};
use crate::trace;

// external crates
use aws_sdk_s3::config::{BehaviorVersion, Credentials as AwsCredentials, Region};
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client;
use tokio::io::AsyncWriteExt;

pub mod errors;
pub mod multipart;

use errors::ObjectNotFoundErr;
pub use errors::S3Err;
pub use multipart::{PartToUpload, Source, UploadedPart};

/// Objects larger than this stream through a multipart upload; objects at or
/// below it go through a single `PutObject`. S3's own multipart part-size
/// floor is 5 MiB; 8 MiB gives headroom while keeping part counts small.
pub(crate) const PART_SIZE: u64 = 8 * 1024 * 1024; // 8 MiB

pub struct Config {
    pub creds: Credentials,
    pub region: String,
}

pub struct Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: String,
}

#[cfg(feature = "test")]
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
    /// The whole file is never held in memory: files at or below [`PART_SIZE`]
    /// stream through one `PutObject` ([`Self::put_singlepart`]); larger files
    /// stream part-by-part through a stateless multipart upload
    /// ([`Self::put_multipart`]).
    pub async fn put(&self, src: File, dst: &Object) -> Result<(), S3Err> {
        let size = files::size(&src).await?;
        if size > PART_SIZE {
            self.put_multipart(&multipart::Source { file: src, size }, dst)
                .await
        } else {
            self.put_singlepart(&src, dst).await
        }
    }

    /// Streams a file to S3 as a single-part upload.
    pub async fn put_singlepart(&self, src: &File, dst: &Object) -> Result<(), S3Err> {
        let body = ByteStream::from_path(src.path())
            .await
            .map_err(|e| errors::map_bytestream_err("put_object", dst, src, &e))?;
        self.client
            .put_object()
            .bucket(&dst.bucket)
            .key(&dst.key)
            .body(body)
            .send()
            .await
            .map_err(|e| errors::map_sdk_err("put_object", Some(dst.key.to_string()), e))?;
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
                if errors::is_not_found(&err) {
                    return Err(S3Err::ObjectNotFoundErr(ObjectNotFoundErr {
                        key: src.key.to_string(),
                        trace: trace!(),
                    }));
                }
                return Err(errors::map_sdk_err(
                    "get_object",
                    Some(src.key.to_string()),
                    err,
                ));
            }
        };

        // Stream the body straight to `dest`, chunk by chunk, so a body-read
        // failure (a retryable transport error) is classified distinctly from a
        // local write failure. `File::create` truncates any existing file. On
        // failure a partially-written `dest` may remain; cleaning that up is the
        // caller's responsibility.
        let mut body = output.body;
        let mut file = tokio::fs::File::create(dest.path())
            .await
            .map_err(|e| errors::map_body_io_err("get_object", src, dest, e))?;
        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(|e| errors::map_body_read_err("get_object", &src.key, &e))?;
            file.write_all(&chunk)
                .await
                .map_err(|e| errors::map_body_io_err("get_object", src, dest, e))?;
        }
        file.flush()
            .await
            .map_err(|e| errors::map_body_io_err("get_object", src, dest, e))?;
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
            .map_err(|e| errors::map_sdk_err("delete_object", Some(obj.key.to_string()), e))?;
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
                    Err(errors::map_sdk_err(
                        "head_object",
                        Some(obj.key.to_string()),
                        err,
                    ))
                }
            }
        }
    }
}
