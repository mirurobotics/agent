// standard crates
use std::sync::Arc;

// internal crates
use crate::authn;
use crate::calibrations;
use crate::disk::Storage;
use crate::events;
use crate::http;
use crate::sync;

// external crates
use tokio::sync::broadcast;

#[derive(Clone, Debug)]
pub struct State {
    pub storage: Arc<Storage>,
    pub http_client: Arc<http::Client>,
    pub syncer: Arc<sync::Syncer>,
    pub token_mngr: Arc<authn::TokenManager>,
    pub event_hub: events::EventHub,
    pub calibration_uploader: Arc<calibrations::Uploader>,
    pub shutdown_tx: broadcast::Sender<()>,
}

impl State {
    pub fn new(
        storage: Arc<Storage>,
        http_client: Arc<http::Client>,
        syncer: Arc<sync::Syncer>,
        token_mngr: Arc<authn::TokenManager>,
        event_hub: events::EventHub,
        calibration_uploader: Arc<calibrations::Uploader>,
        shutdown_tx: broadcast::Sender<()>,
    ) -> Self {
        State {
            storage,
            http_client,
            syncer,
            token_mngr,
            event_hub,
            calibration_uploader,
            shutdown_tx,
        }
    }
}
