// standard crates
use std::future::Future;

// internal crates
use crate::filesys::File;
use crate::s3;
use crate::trace;
use crate::upload::errors::{ExecutorErr, UploadErr};
use backend_api::models::upload_credentials::Scheme;
use backend_api::models::{S3UploadCredentials, UploadCredentials, UploadDestination};

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
/// `endpoint`); GCS is not implemented yet.
pub struct SdkTransfer;

impl ObjectTransfer for SdkTransfer {
    async fn transfer(
        &self,
        credentials: &UploadCredentials,
        destination: &UploadDestination,
        file: &File,
    ) -> Result<(), UploadErr> {
        match credentials.scheme {
            Scheme::S3 => transfer_s3(credentials, destination, file).await,
            Scheme::Gcs => Err(unsupported("GCS uploads are not yet implemented")),
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
    store.put(file.clone(), &object).await.map_err(|e| {
        UploadErr::ExecutorErr(ExecutorErr {
            source: Box::new(e),
            trace: trace!(),
        })
    })
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

/// Builds an [`ExecutorErr`] from a static reason, for credential/scheme cases
/// the device cannot act on.
fn unsupported(msg: &str) -> UploadErr {
    UploadErr::ExecutorErr(ExecutorErr {
        source: msg.to_string().into(),
        trace: trace!(),
    })
}
