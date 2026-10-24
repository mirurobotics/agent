pub mod aws;
mod body;
pub mod errors;
pub mod gcs;
pub mod multipart;
pub mod store;

pub use self::aws::{Config, Credentials};
pub use self::errors::S3Err;
pub use self::store::{Object, Options, Source, Store};
