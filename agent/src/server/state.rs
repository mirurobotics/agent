// standard library
use std::sync::Arc;

// internal crates
use crate::activity;
use crate::authn::token_mngr::TokenManager;
use crate::http;
use crate::storage::{Caches, Device};
use crate::sync::syncer::Syncer;

#[derive(Clone, Debug)]
pub struct ServerState {
    pub device_stor: Arc<Device>,
    pub http_client: Arc<http::Client>,
    pub syncer: Arc<Syncer>,
    pub caches: Arc<Caches>,
    pub token_mngr: Arc<TokenManager>,
    pub activity_tracker: Arc<activity::Tracker>,
}

impl ServerState {
    pub fn new(
        device_stor: Arc<Device>,
        http_client: Arc<http::Client>,
        syncer: Arc<Syncer>,
        caches: Arc<Caches>,
        token_mngr: Arc<TokenManager>,
        activity_tracker: Arc<activity::Tracker>,
    ) -> Self {
        ServerState {
            device_stor,
            http_client,
            syncer,
            caches,
            token_mngr,
            activity_tracker,
        }
    }
}
