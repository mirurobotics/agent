// standard library
use std::sync::Arc;

// internal crates
use crate::activity;
use crate::authn::token_mngr::TokenManager;
use crate::http::client::HTTPClient;
use crate::storage::{caches::Caches, device::DeviceFile};
use crate::sync::syncer::Syncer;

#[derive(Clone, Debug)]
pub struct ServerState {
    pub device_file: Arc<DeviceFile>,
    pub http_client: Arc<HTTPClient>,
    pub syncer: Arc<Syncer>,
    pub caches: Arc<Caches>,
    pub token_mngr: Arc<TokenManager>,
    pub activity_tracker: Arc<activity::Tracker>,
}

impl ServerState {
    pub fn new(
        device_file: Arc<DeviceFile>,
        http_client: Arc<HTTPClient>,
        syncer: Arc<Syncer>,
        caches: Arc<Caches>,
        token_mngr: Arc<TokenManager>,
        activity_tracker: Arc<activity::Tracker>,
    ) -> Self {
        ServerState {
            device_file,
            http_client,
            syncer,
            caches,
            token_mngr,
            activity_tracker,
        }
    }
}
