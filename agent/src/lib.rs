pub mod activity;
pub mod app;
pub mod authn;
pub mod cache;
pub mod cli;
pub mod cooldown;
pub mod crypt;
pub mod data_uploads;
pub mod deploy;
pub mod disk;
pub mod errors;
pub mod events;
pub mod filesys;
pub mod gcs;
pub mod http;
pub mod logs;
pub mod models;
pub mod mqtt;
pub mod network;
pub mod platform;
pub mod privilege;
pub mod provisioning;
pub mod s3;
pub mod server;
pub mod services;
pub mod sync;
pub mod telemetry;
pub mod version;
pub mod windows;
pub mod workers;

// Fixture sources under `agent/tests/test_utils/` name the library as
// `miru_agent`, so this self-alias lets the same files be mounted here for
// inline unit tests. The mount stays `pub` (not `pub(crate)`) so fixtures used
// only by the integration crate do not raise dead-code warnings in the unit build.
#[cfg(test)]
extern crate self as miru_agent;

#[cfg(test)]
#[path = "../tests/test_utils/unit.rs"]
pub mod test_utils;
