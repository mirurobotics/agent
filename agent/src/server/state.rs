// standard library
use std::sync::Arc;

// internal crates
use crate::activity;
use crate::authn::token_mngr::TokenManager;
use crate::http;
use crate::storage::Storage;
use crate::sync::syncer::Syncer;

#[derive(Clone, Debug)]
pub struct ServerState {
    pub storage: Arc<Storage>,
    pub http_client: Arc<http::Client>,
    pub syncer: Arc<Syncer>,
    pub token_mngr: Arc<TokenManager>,
    pub activity_tracker: Arc<activity::Tracker>,
}

impl ServerState {
    pub fn new(
        storage: Arc<Storage>,
        http_client: Arc<http::Client>,
        syncer: Arc<Syncer>,
        token_mngr: Arc<TokenManager>,
        activity_tracker: Arc<activity::Tracker>,
    ) -> Self {
        ServerState {
            storage,
            http_client,
            syncer,
            token_mngr,
            activity_tracker,
        }
    }
}
