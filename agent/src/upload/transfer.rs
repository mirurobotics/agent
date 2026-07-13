// standard crates
use std::future::Future;

// internal crates
use crate::filesys::File;
use crate::gcs;
use crate::s3;
use crate::trace;
use crate::upload::errors::{ExecutorErr, UploadErr};
use backend_api::models::upload_credentials::Scheme;
use backend_api::models::{
    GcsUploadCredentials, S3UploadCredentials, UploadCredentials, UploadDestination,
};

/// The seam between [`BrokerExecutor`](crate::upload::BrokerExecutor) and the
/// concrete cloud-storage SDKs. Given the vended downscoped credentials and the
/// server-authorized destination, transfer the file's bytes to the object
/// store. Kept separate from the executor so the executor's orchestration
/// (create, dedup, confirm) is unit-testable without a live object store.
///
/// # Cancel safety
///
/// Called from the upload actor, whose in-flight future may be dropped on
/// shutdown, so implementations must tolerate cancellation at any await point.
pub trait ObjectTransfer: Send + Sync {
    fn transfer(
        &self,
        credentials: &UploadCredentials,
        destination: &UploadDestination,
        file: &File,
    ) -> impl Future<Output = Result<(), UploadErr>> + Send;
}

/// Production [`ObjectTransfer`] that drives the native cloud SDKs. The `s3`
/// scheme runs an AWS S3 single-part/multipart `PutObject` (also serving
/// S3-compatible stores such as Cloudflare R2 and MinIO via the credential
/// `endpoint`); the `gcs` scheme runs a native GCS upload driven by the vended
/// downscoped OAuth2 bearer token.
#[derive(Default)]
pub struct SdkTransfer {
    /// Test-only override pointing the GCS data client at a local mock
    /// server. Always `None` in production builds, where the field does
    /// not exist and the real GCS endpoint is used.
    #[cfg(feature = "test")]
    pub gcs_endpoint: Option<String>,
}

impl ObjectTransfer for SdkTransfer {
    async fn transfer(
        &self,
        credentials: &UploadCredentials,
        destination: &UploadDestination,
        file: &File,
    ) -> Result<(), UploadErr> {
        match credentials.scheme {
            Scheme::S3 => transfer_s3(credentials, destination, file).await,
            Scheme::Gcs => {
                #[cfg(feature = "test")]
                let endpoint = self.gcs_endpoint.clone();
                #[cfg(not(feature = "test"))]
                let endpoint = None;
                transfer_gcs(credentials, destination, file, endpoint).await
            }
            Scheme::SchemeUnknown => Err(unsupported("unrecognized upload credential scheme")),
        }
    }
}

/// Uploads `file` to the S3-compatible object store named by `destination`
/// using the vended session credentials. The physical bucket comes from
/// `destination.bucket_name` (not the Miru `bucket_id`); the endpoint and
/// region come from the credentials so the same path serves AWS, R2, and MinIO.
async fn transfer_s3(
    credentials: &UploadCredentials,
    destination: &UploadDestination,
    file: &File,
) -> Result<(), UploadErr> {
    let creds = credentials
        .s3_credentials
        .as_deref()
        .ok_or_else(|| unsupported("s3 scheme is missing s3_credentials"))?;
    let store = s3::Store::new(s3_config(creds));
    let object = s3::Object {
        bucket: destination.bucket_name.clone(),
        key: destination.object_key.clone(),
    };
    store.put(file.clone(), &object).await.map_err(executor_err)
}

/// Uploads `file` to the GCS bucket named by `destination` using the vended
/// downscoped OAuth2 bearer token. The physical bucket comes from
/// `destination.bucket_name` (not the Miru `bucket_id`); the credentials carry
/// no endpoint or region — the token alone scopes the store to the object.
async fn transfer_gcs(
    credentials: &UploadCredentials,
    destination: &UploadDestination,
    file: &File,
    endpoint: Option<String>,
) -> Result<(), UploadErr> {
    let creds = credentials
        .gcs_credentials
        .as_deref()
        .ok_or_else(|| unsupported("gcs scheme is missing gcs_credentials"))?;
    let store = gcs_store(gcs_credentials(creds), endpoint)
        .await
        .map_err(executor_err)?;
    let object = gcs::Object {
        bucket: destination.bucket_name.clone(),
        key: destination.object_key.clone(),
    };
    store.put(file.clone(), &object).await.map_err(executor_err)
}

/// Builds the GCS store; the `endpoint` override (test builds only) points the
/// data client at a local mock server.
async fn gcs_store(
    creds: gcs::Credentials,
    endpoint: Option<String>,
) -> Result<gcs::Store, gcs::GcsErr> {
    let _ = &endpoint; // consumed only in test builds
    #[cfg(feature = "test")]
    if let Some(ep) = endpoint {
        return gcs::Store::from_endpoint(creds, ep).await;
    }
    gcs::Store::new(creds).await
}

/// Maps vended S3 session credentials into an [`s3::Config`]. The endpoint and
/// region come from the credentials so one path serves AWS, Cloudflare R2, and
/// MinIO. Kept separate so the credential→SDK mapping is unit-testable without a
/// live transfer.
pub fn s3_config(creds: &S3UploadCredentials) -> s3::Config {
    s3::Config {
        creds: s3::Credentials {
            access_key_id: creds.access_key_id.clone(),
            secret_access_key: creds.secret_access_key.clone(),
            session_token: creds.session_token.clone(),
        },
        region: creds.region.clone(),
        endpoint: Some(creds.endpoint.clone()),
    }
}

/// Maps vended GCS credentials into [`gcs::Credentials`]. Kept separate so the
/// credential→SDK mapping is unit-testable without a live transfer. The
/// credentials' `expires_at` is deliberately not consumed here — mid-upload
/// expiry handling belongs to the executor.
pub fn gcs_credentials(creds: &GcsUploadCredentials) -> gcs::Credentials {
    gcs::Credentials {
        access_token: creds.access_token.clone(),
    }
}

/// Wraps any transfer-layer error into [`UploadErr::ExecutorErr`].
fn executor_err<E: std::error::Error + Send + Sync + 'static>(e: E) -> UploadErr {
    UploadErr::ExecutorErr(ExecutorErr {
        source: Box::new(e),
        trace: trace!(),
    })
}

/// Builds an [`ExecutorErr`] from a static reason, for credential/scheme cases
/// the device cannot act on.
fn unsupported(msg: &str) -> UploadErr {
    UploadErr::ExecutorErr(ExecutorErr {
        source: msg.to_string().into(),
        trace: trace!(),
    })
}
