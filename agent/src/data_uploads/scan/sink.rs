// standard crates
use std::future::Future;
use std::pin::Pin;

// internal crates
use crate::data_uploads::scan::scanner::StableFile;
use crate::models::FileRule;

pub trait StableFileSink: Send + Sync {
    fn on_stable_file<'a>(
        &'a self,
        file: StableFile,
        rule: &'a FileRule,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}
