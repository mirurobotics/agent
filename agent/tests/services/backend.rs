// standard crates
use std::sync::atomic::{AtomicUsize, Ordering};

// internal crates
use crate::mocks::{
    http_client::{Call, CapturedRequest, MockClient},
    stub_token_manager::StubTokenManager,
};
use backend_api::models as backend_client;
use miru_agent::authn::errors::{AuthnErr, MockError as AuthnMockError};
use miru_agent::http::errors::{HTTPErr, MockErr as HttpMockErr, RequestFailed};
use miru_agent::http::request::Params as HttpParams;
use miru_agent::services::{BackendFetcher, HttpBackend, ServiceErr};
use miru_agent::sync::SyncErr;

#[tokio::test]
async fn fetch_deployment_constructs_url_and_expand_param() {
    let mock = MockClient::default();
    let token_mngr = StubTokenManager::ok("test-token");
    let backend = HttpBackend::new(&mock, &token_mngr);

    backend.fetch_deployment("dpl_1").await.unwrap();

    let expected = CapturedRequest {
        call: Call::GetDeployment,
        method: reqwest::Method::GET,
        path: "/deployments/dpl_1".to_string(),
        url: "http://mock/deployments/dpl_1".to_string(),
        query: vec![("expand".to_string(), "config_instances".to_string())],
        body: None,
        token: Some("test-token".to_string()),
    };
    assert_eq!(mock.requests(), vec![expected]);
}

#[tokio::test]
async fn fetch_deployment_returns_deserialized_value() {
    let mock = MockClient::default();
    let expected = backend_client::Deployment {
        id: "dpl_1".to_string(),
        description: "test".to_string(),
        device_id: "dvc_1".to_string(),
        release_id: "rls_1".to_string(),
        ..Default::default()
    };
    mock.set_get_deployment({
        let expected = expected.clone();
        move || Ok(expected.clone())
    });
    let token_mngr = StubTokenManager::ok("test-token");
    let backend = HttpBackend::new(&mock, &token_mngr);

    let dpl = backend.fetch_deployment("dpl_1").await.unwrap();
    assert_eq!(dpl, expected);
}

#[tokio::test]
async fn fetch_release_constructs_url_no_expand() {
    let mock = MockClient::default();
    let token_mngr = StubTokenManager::ok("test-token");
    let backend = HttpBackend::new(&mock, &token_mngr);

    backend.fetch_release("rls_1").await.unwrap();

    let expected = CapturedRequest {
        call: Call::GetRelease,
        method: reqwest::Method::GET,
        path: "/releases/rls_1".to_string(),
        url: "http://mock/releases/rls_1".to_string(),
        query: vec![],
        body: None,
        token: Some("test-token".to_string()),
    };
    assert_eq!(mock.requests(), vec![expected]);
}

#[tokio::test]
async fn fetch_release_returns_deserialized_value() {
    let mock = MockClient::default();
    mock.set_get_release(|| {
        Ok(backend_client::Release {
            id: "rls_1".to_string(),
            version: "1.2.3".to_string(),
            git_commit_id: Some("gc_1".to_string()),
            ..Default::default()
        })
    });
    let token_mngr = StubTokenManager::ok("test-token");
    let backend = HttpBackend::new(&mock, &token_mngr);

    let rls = backend.fetch_release("rls_1").await.unwrap();
    assert_eq!(rls.id, "rls_1");
    assert_eq!(rls.version, "1.2.3");
    assert_eq!(rls.git_commit_id, Some("gc_1".to_string()));
}

#[tokio::test]
async fn fetch_git_commit_constructs_url_no_expand() {
    let mock = MockClient::default();
    let token_mngr = StubTokenManager::ok("test-token");
    let backend = HttpBackend::new(&mock, &token_mngr);

    backend.fetch_git_commit("gc_1").await.unwrap();

    let expected = CapturedRequest {
        call: Call::GetGitCommit,
        method: reqwest::Method::GET,
        path: "/git_commits/gc_1".to_string(),
        url: "http://mock/git_commits/gc_1".to_string(),
        query: vec![],
        body: None,
        token: Some("test-token".to_string()),
    };
    assert_eq!(mock.requests(), vec![expected]);
}

#[tokio::test]
async fn fetch_git_commit_returns_deserialized_value() {
    let mock = MockClient::default();
    mock.set_get_git_commit(|| {
        Ok(backend_client::GitCommit {
            id: "gc_1".to_string(),
            sha: "abc123".to_string(),
            message: "initial commit".to_string(),
            ..Default::default()
        })
    });
    let token_mngr = StubTokenManager::ok("test-token");
    let backend = HttpBackend::new(&mock, &token_mngr);

    let gc = backend.fetch_git_commit("gc_1").await.unwrap();
    assert_eq!(gc.id, "gc_1");
    assert_eq!(gc.sha, "abc123");
    assert_eq!(gc.message, "initial commit");
}

#[tokio::test]
async fn fetch_deployment_token_failure_returns_sync_err() {
    let mock = MockClient::default();
    let token_mngr = StubTokenManager::err(AuthnErr::MockError(AuthnMockError {
        is_network_conn_err: false,
        trace: miru_agent::trace!(),
    }));
    let backend = HttpBackend::new(&mock, &token_mngr);

    let result = backend.fetch_deployment("dpl_1").await;
    assert!(matches!(
        result,
        Err(ServiceErr::SyncErr(SyncErr::AuthnErr(AuthnErr::MockError(
            _
        ))))
    ));
    // No HTTP request should have been issued.
    assert!(mock.requests().is_empty());
}

#[tokio::test]
async fn fetch_deployment_404_propagates_as_request_failed() {
    let mock = MockClient::default();
    mock.set_get_deployment(|| {
        Err(HTTPErr::RequestFailed(RequestFailed {
            request: HttpParams::get("http://mock/deployments/dpl_1")
                .meta()
                .unwrap(),
            status: reqwest::StatusCode::NOT_FOUND,
            error: None,
            trace: miru_agent::trace!(),
        }))
    });
    let token_mngr = StubTokenManager::ok("test-token");
    let backend = HttpBackend::new(&mock, &token_mngr);

    let result = backend.fetch_deployment("dpl_1").await;
    let err = result.expect_err("expected 404 to propagate as error");
    match err {
        ServiceErr::HTTPErr(HTTPErr::RequestFailed(rf)) => {
            assert_eq!(rf.status, reqwest::StatusCode::NOT_FOUND);
        }
        other => panic!("expected ServiceErr::HTTPErr(RequestFailed), got {other:?}"),
    }
}

#[tokio::test]
async fn fetch_deployment_5xx_propagates_as_request_failed() {
    let mock = MockClient::default();
    mock.set_get_deployment(|| {
        Err(HTTPErr::RequestFailed(RequestFailed {
            request: HttpParams::get("http://mock/deployments/dpl_1")
                .meta()
                .unwrap(),
            status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            error: None,
            trace: miru_agent::trace!(),
        }))
    });
    let token_mngr = StubTokenManager::ok("test-token");
    let backend = HttpBackend::new(&mock, &token_mngr);

    let result = backend.fetch_deployment("dpl_1").await;
    let err = result.expect_err("expected 5xx to propagate as error");
    match err {
        ServiceErr::HTTPErr(HTTPErr::RequestFailed(rf)) => {
            assert_eq!(rf.status, reqwest::StatusCode::INTERNAL_SERVER_ERROR);
        }
        other => panic!("expected ServiceErr::HTTPErr(RequestFailed), got {other:?}"),
    }
}

#[tokio::test]
async fn fetch_deployment_with_retry_recovers_from_network_error() {
    let mock = MockClient::default();
    let counter = AtomicUsize::new(0);
    mock.set_get_deployment(move || {
        let n = counter.fetch_add(1, Ordering::SeqCst);
        if n < 2 {
            Err(HTTPErr::MockErr(HttpMockErr {
                is_network_conn_err: true,
            }))
        } else {
            Ok(backend_client::Deployment {
                id: "dpl_1".to_string(),
                ..Default::default()
            })
        }
    });
    let token_mngr = StubTokenManager::ok("test-token");
    let backend = HttpBackend::new(&mock, &token_mngr);

    let dpl = backend.fetch_deployment("dpl_1").await.unwrap();
    assert_eq!(dpl.id, "dpl_1");
    // Retry logic: 2 failures + 1 success = 3 total calls.
    assert_eq!(mock.call_count(Call::GetDeployment), 3);
}

// ========================== fetch_release error paths ========================== //

#[tokio::test]
async fn fetch_release_token_failure_returns_sync_err() {
    let mock = MockClient::default();
    let token_mngr = StubTokenManager::err(AuthnErr::MockError(AuthnMockError {
        is_network_conn_err: false,
        trace: miru_agent::trace!(),
    }));
    let backend = HttpBackend::new(&mock, &token_mngr);

    let result = backend.fetch_release("rls_1").await;
    assert!(matches!(
        result,
        Err(ServiceErr::SyncErr(SyncErr::AuthnErr(AuthnErr::MockError(
            _
        ))))
    ));
    // No HTTP request should have been issued.
    assert!(mock.requests().is_empty());
}

#[tokio::test]
async fn fetch_release_404_propagates_as_request_failed() {
    let mock = MockClient::default();
    mock.set_get_release(|| {
        Err(HTTPErr::RequestFailed(RequestFailed {
            request: HttpParams::get("http://mock/releases/rls_1")
                .meta()
                .unwrap(),
            status: reqwest::StatusCode::NOT_FOUND,
            error: None,
            trace: miru_agent::trace!(),
        }))
    });
    let token_mngr = StubTokenManager::ok("test-token");
    let backend = HttpBackend::new(&mock, &token_mngr);

    let result = backend.fetch_release("rls_1").await;
    let err = result.expect_err("expected 404 to propagate as error");
    match err {
        ServiceErr::HTTPErr(HTTPErr::RequestFailed(rf)) => {
            assert_eq!(rf.status, reqwest::StatusCode::NOT_FOUND);
        }
        other => panic!("expected ServiceErr::HTTPErr(RequestFailed), got {other:?}"),
    }
}

#[tokio::test]
async fn fetch_release_with_retry_recovers_from_network_error() {
    let mock = MockClient::default();
    let counter = AtomicUsize::new(0);
    mock.set_get_release(move || {
        let n = counter.fetch_add(1, Ordering::SeqCst);
        if n < 2 {
            Err(HTTPErr::MockErr(HttpMockErr {
                is_network_conn_err: true,
            }))
        } else {
            Ok(backend_client::Release {
                id: "rls_1".to_string(),
                ..Default::default()
            })
        }
    });
    let token_mngr = StubTokenManager::ok("test-token");
    let backend = HttpBackend::new(&mock, &token_mngr);

    let rls = backend.fetch_release("rls_1").await.unwrap();
    assert_eq!(rls.id, "rls_1");
    // Retry logic: 2 failures + 1 success = 3 total calls.
    assert_eq!(mock.call_count(Call::GetRelease), 3);
}

// ======================== fetch_git_commit error paths ========================= //

#[tokio::test]
async fn fetch_git_commit_token_failure_returns_sync_err() {
    let mock = MockClient::default();
    let token_mngr = StubTokenManager::err(AuthnErr::MockError(AuthnMockError {
        is_network_conn_err: false,
        trace: miru_agent::trace!(),
    }));
    let backend = HttpBackend::new(&mock, &token_mngr);

    let result = backend.fetch_git_commit("gc_1").await;
    assert!(matches!(
        result,
        Err(ServiceErr::SyncErr(SyncErr::AuthnErr(AuthnErr::MockError(
            _
        ))))
    ));
    // No HTTP request should have been issued.
    assert!(mock.requests().is_empty());
}

#[tokio::test]
async fn fetch_git_commit_404_propagates_as_request_failed() {
    let mock = MockClient::default();
    mock.set_get_git_commit(|| {
        Err(HTTPErr::RequestFailed(RequestFailed {
            request: HttpParams::get("http://mock/git_commits/gc_1")
                .meta()
                .unwrap(),
            status: reqwest::StatusCode::NOT_FOUND,
            error: None,
            trace: miru_agent::trace!(),
        }))
    });
    let token_mngr = StubTokenManager::ok("test-token");
    let backend = HttpBackend::new(&mock, &token_mngr);

    let result = backend.fetch_git_commit("gc_1").await;
    let err = result.expect_err("expected 404 to propagate as error");
    match err {
        ServiceErr::HTTPErr(HTTPErr::RequestFailed(rf)) => {
            assert_eq!(rf.status, reqwest::StatusCode::NOT_FOUND);
        }
        other => panic!("expected ServiceErr::HTTPErr(RequestFailed), got {other:?}"),
    }
}

#[tokio::test]
async fn fetch_git_commit_with_retry_recovers_from_network_error() {
    let mock = MockClient::default();
    let counter = AtomicUsize::new(0);
    mock.set_get_git_commit(move || {
        let n = counter.fetch_add(1, Ordering::SeqCst);
        if n < 2 {
            Err(HTTPErr::MockErr(HttpMockErr {
                is_network_conn_err: true,
            }))
        } else {
            Ok(backend_client::GitCommit {
                id: "gc_1".to_string(),
                ..Default::default()
            })
        }
    });
    let token_mngr = StubTokenManager::ok("test-token");
    let backend = HttpBackend::new(&mock, &token_mngr);

    let gc = backend.fetch_git_commit("gc_1").await.unwrap();
    assert_eq!(gc.id, "gc_1");
    // Retry logic: 2 failures + 1 success = 3 total calls.
    assert_eq!(mock.call_count(Call::GetGitCommit), 3);
}
