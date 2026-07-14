// standard crates
use std::future::Future;

// internal crates
use crate::filesys::File;
use crate::gcs;
use crate::s3;
use crate::trace;
use crate::upload::errors::{ExecutorErr, UploadErr};
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
    fn transfer(
        &self,
        credentials: &UploadCredentials,
        destination: &UploadDestination,
        file: &File,
    ) -> impl Future<Output = Result<(), UploadErr>> + Send;
}

/// Production [`ObjectTransfer`] that drives the native cloud SDKs: an AWS S3
/// single-part/multipart put for the `s3` scheme, and a GCS put (the SDK picks
/// simple vs resumable) with the vended OAuth2 bearer token for `gcs`.
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
            Scheme::Gcs => transfer_gcs(credentials, destination, file).await,
            Scheme::SchemeUnknown => Err(unsupported("unrecognized upload credential scheme")),
        }
    }
}

/// Uploads `file` to AWS S3 using the vended session credentials. The physical
/// bucket comes from `destination.bucket_name` (not the Miru `bucket_id`); the
/// SDK derives the endpoint from the credential `region`.
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

/// Uploads `file` to GCS using the vended downscoped OAuth2 bearer token,
/// which is scoped to `storage.objects.create` on this exact object.
async fn transfer_gcs(
    credentials: &UploadCredentials,
    destination: &UploadDestination,
    file: &File,
) -> Result<(), UploadErr> {
    let creds = credentials
        .gcs_credentials
        .as_deref()
        .ok_or_else(|| unsupported("gcs scheme is missing gcs_credentials"))?;
    let store = gcs::Store::new(gcs::Credentials {
        access_token: creds.access_token.clone(),
    })
    .await
    .map_err(executor_err)?;
    let object = gcs::Object {
        bucket: destination.bucket_name.clone(),
        key: destination.object_key.clone(),
    };
    store.put(file.clone(), &object).await.map_err(executor_err)
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

/// Wraps any concrete error as an [`UploadErr::ExecutorErr`], the single error
/// surface the actor sees from transfer failures.
fn executor_err<E>(source: E) -> UploadErr
where
    E: std::error::Error + Send + Sync + 'static,
{
    UploadErr::ExecutorErr(ExecutorErr {
        source: Box::new(source),
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
