// standard crates
use std::sync::Arc;

// internal crates
use crate::activity;
use crate::authn;
use crate::events;
use crate::http;
use crate::storage::Storage;
use crate::sync;

// external crates
use tokio::sync::broadcast;

#[derive(Clone, Debug)]
pub struct State {
    pub storage: Arc<Storage>,
    pub http_client: Arc<http::Client>,
    pub syncer: Arc<sync::Syncer>,
    pub token_mngr: Arc<authn::TokenManager>,
    pub activity_tracker: Arc<activity::Tracker>,
    pub event_hub: events::EventHub,
    pub shutdown_tx: broadcast::Sender<()>,
}

impl State {
    pub fn new(
        storage: Arc<Storage>,
        http_client: Arc<http::Client>,
        syncer: Arc<sync::Syncer>,
        token_mngr: Arc<authn::TokenManager>,
        activity_tracker: Arc<activity::Tracker>,
        event_hub: events::EventHub,
        shutdown_tx: broadcast::Sender<()>,
    ) -> Self {
        State {
            storage,
            http_client,
            syncer,
            token_mngr,
            activity_tracker,
            event_hub,
            shutdown_tx,
        }
    }
}
