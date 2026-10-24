pub mod errors;
pub mod gcs;
pub mod multipart;
pub mod store;

pub use self::errors::S3Err;
pub use self::store::{Config, Credentials, Object, Source, Store};
