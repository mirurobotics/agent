// standard crates
use std::future::Future;

// internal crates
use crate::filesys::File;
use crate::gcs;
use crate::s3;
use crate::upload::errors::{executor_err, UploadErr};
use backend_api::models::upload_credentials::Scheme;
use backend_api::models::{S3UploadCredentials, UploadCredentials, UploadDestination};

/// The seam between the upload executor and the concrete cloud-storage SDKs.
/// Given the vended downscoped credentials and the server-authorized
/// destination, transfer the file's bytes to the object store. Kept separate
/// from the executor so its orchestration (create, dedup, confirm) is
/// unit-testable without a live object store.
///
/// # Cancel safety
///
/// Called from the upload actor, whose in-flight future may be dropped on
/// shutdown, so implementations must tolerate cancellation at any await point.
pub trait ObjectTransfer: Send + Sync {
    /// `expected_crc32c` is the scanner's CRC32C of the content, forwarded to the
    /// object store as the integrity check it enforces on upload (currently the
    /// GCS scheme; `None` skips the check). See [`crate::gcs::Store::put`].
    fn transfer(
        &self,
        credentials: &UploadCredentials,
        destination: &UploadDestination,
        file: &File,
        expected_crc32c: Option<u32>,
    ) -> impl Future<Output = Result<(), UploadErr>> + Send;
}

/// Production [`ObjectTransfer`] that drives the native cloud SDKs: an AWS S3
/// single-part/multipart put for the `s3` scheme, and a GCS put (the SDK picks
/// simple vs resumable) with the vended OAuth2 bearer token for `gcs`. Under
/// the `test` feature the store construction can be overridden — an injected
/// S3 HTTP client or a local GCS endpoint — so tests drive the full transfer
/// path offline; `Default` leaves both unset for the production behavior.
#[derive(Default)]
pub struct SdkTransfer {
    /// Test-only: injected in place of the default HTTPS connector, so the
    /// full transfer path runs against a replayed S3 exchange offline.
    #[cfg(feature = "test")]
    s3_http_client: Option<aws_sdk_s3::config::SharedHttpClient>,
    /// Test-only: points the GCS data client at a local mock server.
    #[cfg(feature = "test")]
    gcs_endpoint: Option<String>,
}

impl SdkTransfer {
    /// Test-only constructor that injects a caller-provided HTTP client (e.g. a
    /// `StaticReplayClient`) into the S3 store in place of the default HTTPS
    /// connector, so tests serve canned responses without touching the network.
    #[cfg(feature = "test")]
    pub fn with_s3_http_client(http_client: impl aws_sdk_s3::config::HttpClient + 'static) -> Self {
        Self {
            s3_http_client: Some(aws_sdk_s3::config::IntoShared::into_shared(http_client)),
            gcs_endpoint: None,
        }
    }

    /// Test-only constructor that points the GCS store at a local mock server
    /// instead of the real GCS endpoint, so tests serve canned responses
    /// without touching the network.
    #[cfg(feature = "test")]
    pub fn with_gcs_endpoint(endpoint: String) -> Self {
        Self {
            s3_http_client: None,
            gcs_endpoint: Some(endpoint),
        }
    }

    /// Uploads `file` to AWS S3 using the vended session credentials. The physical
    /// bucket comes from `destination.bucket_name` (not the Miru `bucket_id`); the
    /// SDK derives the endpoint from the credential `region`.
    async fn transfer_s3(
        &self,
        credentials: &UploadCredentials,
        destination: &UploadDestination,
        file: &File,
    ) -> Result<(), UploadErr> {
        let creds = credentials
            .s3_credentials
            .as_deref()
            .ok_or_else(|| executor_err("s3 scheme is missing s3_credentials"))?;
        let store = self.s3_store(s3_config(creds));
        let object = s3::Object {
            bucket: destination.bucket_name.clone(),
            key: destination.object_key.clone(),
        };
        store.put(file.clone(), &object).await.map_err(executor_err)
    }

    /// Uploads `file` to GCS using the vended downscoped OAuth2 bearer token,
    /// which is scoped to `storage.objects.create` on this exact object.
    /// `expected_crc32c` is forwarded as the checksum GCS enforces on the upload.
    async fn transfer_gcs(
        &self,
        credentials: &UploadCredentials,
        destination: &UploadDestination,
        file: &File,
        expected_crc32c: Option<u32>,
    ) -> Result<(), UploadErr> {
        let creds = credentials
            .gcs_credentials
            .as_deref()
            .ok_or_else(|| executor_err("gcs scheme is missing gcs_credentials"))?;
        let store = self
            .gcs_store(gcs::Credentials {
                access_token: creds.access_token.clone(),
            })
            .await
            .map_err(executor_err)?;
        let object = gcs::Object {
            bucket: destination.bucket_name.clone(),
            key: destination.object_key.clone(),
        };
        store
            .put(file.clone(), &object, expected_crc32c)
            .await
            .map_err(executor_err)
    }

    /// Builds the S3 store, honoring the test-only HTTP client override.
    fn s3_store(&self, cfg: s3::Config) -> s3::Store {
        #[cfg(feature = "test")]
        if let Some(http_client) = &self.s3_http_client {
            return s3::Store::from_http_client(http_client.clone(), cfg);
        }
        s3::Store::new(cfg)
    }

    /// Builds the GCS store, honoring the test-only endpoint override.
    async fn gcs_store(&self, creds: gcs::Credentials) -> Result<gcs::Store, gcs::GcsErr> {
        #[cfg(feature = "test")]
        if let Some(endpoint) = &self.gcs_endpoint {
            return gcs::Store::from_endpoint(creds, endpoint.clone()).await;
        }
        gcs::Store::new(creds).await
    }
}

impl ObjectTransfer for SdkTransfer {
    async fn transfer(
        &self,
        credentials: &UploadCredentials,
        destination: &UploadDestination,
        file: &File,
        expected_crc32c: Option<u32>,
    ) -> Result<(), UploadErr> {
        match credentials.scheme {
            // S3 checksum enforcement is a separate change; `expected_crc32c` is
            // GCS's integrity input (S3 uses SHA-256, not CRC32C).
            Scheme::S3 => self.transfer_s3(credentials, destination, file).await,
            Scheme::Gcs => {
                self.transfer_gcs(credentials, destination, file, expected_crc32c)
                    .await
            }
            Scheme::SchemeUnknown => Err(executor_err("unrecognized upload credential scheme")),
        }
    }
}

/// Maps vended S3 session credentials into an [`s3::Config`]. Kept separate so
/// the credential→SDK mapping is unit-testable without a live transfer.
pub fn s3_config(creds: &S3UploadCredentials) -> s3::Config {
    s3::Config {
        creds: s3::Credentials {
            access_key_id: creds.access_key_id.clone(),
            secret_access_key: creds.secret_access_key.clone(),
            session_token: creds.session_token.clone(),
        },
        region: creds.region.clone(),
    }
}
