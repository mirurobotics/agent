// internal crates
use miru_agent::http::errors::{HTTPErr, MockErr as HttpMockErr, RequestFailed};
use miru_agent::http::request::Params as HttpParams;
use miru_agent::upload::errors::{is_permanent, ExecutorErr, QueueFullErr, SendActorMessageErr};
use miru_agent::upload::UploadErr;

fn request_failed(status: reqwest::StatusCode) -> HTTPErr {
    HTTPErr::RequestFailed(RequestFailed {
        request: HttpParams::get("http://test/uploads").meta().unwrap(),
        status,
        error: None,
        trace: miru_agent::trace!(),
    })
}

fn queue_full_err() -> QueueFullErr {
    QueueFullErr {
        capacity: 1,
        file: "/data/a.log".to_string(),
        trace: miru_agent::trace!(),
    }
}

fn scripted_executor_err(permanent: bool) -> UploadErr {
    UploadErr::ExecutorErr(ExecutorErr {
        source: Box::new(std::io::Error::other("scripted failure")),
        permanent,
        trace: miru_agent::trace!(),
    })
}

#[test]
fn http_statuses_classify_by_permanence() {
    let cases = [
        // definitive client errors are permanent (404 is the incident case)
        (reqwest::StatusCode::NOT_FOUND, true),
        (reqwest::StatusCode::BAD_REQUEST, true),
        // timeout, rate-limit, and stale-token conditions stay retryable
        (reqwest::StatusCode::REQUEST_TIMEOUT, false),
        (reqwest::StatusCode::TOO_MANY_REQUESTS, false),
        (reqwest::StatusCode::UNAUTHORIZED, false),
        // server errors stay retryable
        (reqwest::StatusCode::INTERNAL_SERVER_ERROR, false),
        (reqwest::StatusCode::SERVICE_UNAVAILABLE, false),
    ];
    for (status, expected) in cases {
        assert_eq!(
            is_permanent(&request_failed(status)),
            expected,
            "status {status}"
        );
    }
}

#[test]
fn network_conn_err_is_not_permanent() {
    let err = HTTPErr::MockErr(HttpMockErr {
        is_network_conn_err: true,
    });
    assert!(!is_permanent(&err));
}

#[test]
fn non_http_err_is_not_permanent() {
    // errors on the trait defaults report a 500 status, so they stay retryable
    assert!(!is_permanent(&queue_full_err()));
}

#[test]
fn upload_err_is_permanent_only_for_permanent_executor_err() {
    assert!(scripted_executor_err(true).is_permanent());
    assert!(!scripted_executor_err(false).is_permanent());
    assert!(!UploadErr::QueueFullErr(queue_full_err()).is_permanent());

    let err = UploadErr::SendActorMessageErr(SendActorMessageErr {
        source: Box::new(std::io::Error::other("scripted send failure")),
        trace: miru_agent::trace!(),
    });
    assert!(!err.is_permanent());
}
