pub mod config_instance;
pub mod deployment;
pub mod device;
pub mod errors;
pub mod release;

pub trait Mergeable<UpdatesT> {
    fn merge(&mut self, updates: UpdatesT);
}
