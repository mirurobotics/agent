// standard crates
use std::future::Future;
use std::sync::Arc;

// internal crates
use crate::authn::TokenManagerExt;
use crate::calibrations::{errors::CalibrationsErr, report::Report};
use crate::http::{self, ClientI, HTTPErr};
use backend_api::models::{CreateCalibrationRequest, InstanceContent};

// external crates
use tracing::warn;

/// The seam between the upload worker and Miru.
///
/// # Cancel safety
///
/// The worker drops an in-flight `send` on shutdown. The report is removed
/// from the queue only after `send` returns `Ok`, so an interrupted send is
/// retried on the next boot with the report's own `report_id`, which Miru
/// records once.
pub trait Sender: Send + Sync {
    fn send(&self, report: Report) -> impl Future<Output = Result<(), CalibrationsErr>> + Send;
}

/// Sends each report as one `POST /calibrations`, with no retry of its own:
/// the worker's backoff is the only retry loop.
pub struct HttpSender<C: ClientI, T: TokenManagerExt> {
    http_client: Arc<C>,
    token_mngr: Arc<T>,
}

impl<C: ClientI, T: TokenManagerExt> HttpSender<C, T> {
    pub fn new(http_client: Arc<C>, token_mngr: Arc<T>) -> Self {
        Self {
            http_client,
            token_mngr,
        }
    }
}

impl<C: ClientI, T: TokenManagerExt> Sender for HttpSender<C, T> {
    async fn send(&self, report: Report) -> Result<(), CalibrationsErr> {
        let payload = new_request(report);
        let token = self.token_mngr.get_token().await?;
        let params = http::calibrations::CreateParams {
            payload: &payload,
            token: &token.token,
        };
        match http::calibrations::create(self.http_client.as_ref(), params).await {
            Ok(_) => Ok(()),
            // only a 2xx response body is parsed, so Miru has stored the
            // report and there is nothing to retry
            Err(HTTPErr::UnmarshalJSONErr(e)) => {
                warn!("calibrations: Miru accepted a report but its response could not be parsed: {e}");
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }
}

/// The backend request for `report`. It takes `report` by value so the data
/// moves into the request instead of being copied.
pub fn new_request(report: Report) -> CreateCalibrationRequest {
    let metadata = report.metadata;
    let content = InstanceContent {
        format: (&metadata.format).into(),
        data: report.data,
    };
    CreateCalibrationRequest {
        report_id: metadata.report_id,
        deployment_id: metadata.deployment_id,
        component: metadata.component,
        role: metadata.role,
        serial_number: metadata.serial_number,
        reported_by: metadata.reported_by,
        aspect: metadata.aspect,
        content: Box::new(content),
        reported_at: metadata.reported_at.to_rfc3339(),
    }
}
