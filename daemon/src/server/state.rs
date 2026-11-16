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
    pub cal_uploader: Arc<calibrations::Uploader>,
    pub latest_cals: calibrations::LatestStore,
    pub shutdown_tx: broadcast::Sender<()>,
}
