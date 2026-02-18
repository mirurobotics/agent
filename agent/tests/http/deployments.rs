use std::sync::atomic::{AtomicUsize, Ordering};

use crate::http::mock::{CapturedRequest, MockCall, MockClient};
use miru_agent::http::deployments::{self, ListAllParams, ListParams, UpdateParams};
use miru_agent::http::errors::{HTTPErr, MockErr};
use miru_agent::http::query::Page;
use openapi_client::models::{
    Deployment as BackendDeployment, DeploymentActivityStatus, DeploymentList,
    DeploymentListExpansion, UpdateDeploymentRequest,
};

fn mock_err() -> HTTPErr {
    HTTPErr::MockErr(MockErr {
        is_network_connection_error: false,
    })
}

pub mod list {
    use super::*;

    #[tokio::test]
    async fn success() {
        let mock = MockClient::default();
        mock.set_list_all_deployments(move || {
            Ok(vec![BackendDeployment {
                id: "dep_1".to_string(),
                ..BackendDeployment::default()
            }])
        });

        let result = deployments::list(
            &mock,
            ListParams {
                activity_status: &[],
                expansions: &[],
                pagination: &Page::default(),
                token: "test-token",
            },
        )
        .await
        .unwrap();

        let expected = DeploymentList {
            total_count: 1,
            data: vec![BackendDeployment {
                id: "dep_1".to_string(),
                ..BackendDeployment::default()
            }],
            ..DeploymentList::default()
        };
        assert_eq!(result, expected);
        assert_eq!(mock.call_count(MockCall::ListDeployments), 1);
        assert_eq!(
            mock.requests(),
            vec![CapturedRequest {
                method: reqwest::Method::GET,
                url: "http://mock/deployments".into(),
                query: vec![("limit".into(), "10".into()), ("offset".into(), "0".into()),],
                body: None,
                token: Some("test-token".into()),
            }]
        );
    }

    #[tokio::test]
    async fn with_activity_status_filter() {
        let mock = MockClient::default();

        deployments::list(
            &mock,
            ListParams {
                activity_status: &[
                    DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_QUEUED,
                    DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DEPLOYED,
                ],
                expansions: &[],
                pagination: &Page::default(),
                token: "test-token",
            },
        )
        .await
        .unwrap();

        assert_eq!(
            mock.requests(),
            vec![CapturedRequest {
                method: reqwest::Method::GET,
                url: "http://mock/deployments".into(),
                query: vec![
                    ("limit".into(), "10".into()),
                    ("offset".into(), "0".into()),
                    ("activity_status".into(), "queued,deployed".into()),
                ],
                body: None,
                token: Some("test-token".into()),
            }]
        );
    }

    #[tokio::test]
    async fn with_expansions() {
        let mock = MockClient::default();

        deployments::list(
            &mock,
            ListParams {
                activity_status: &[],
                expansions: &[
                    DeploymentListExpansion::DEPLOYMENT_LIST_EXPAND_RELEASE,
                    DeploymentListExpansion::DEPLOYMENT_LIST_EXPAND_CONFIG_INSTANCES,
                ],
                pagination: &Page::default(),
                token: "test-token",
            },
        )
        .await
        .unwrap();

        assert_eq!(
            mock.requests(),
            vec![CapturedRequest {
                method: reqwest::Method::GET,
                url: "http://mock/deployments".into(),
                query: vec![
                    ("limit".into(), "10".into()),
                    ("offset".into(), "0".into()),
                    ("expand".into(), "release".into()),
                    ("expand".into(), "config_instances".into()),
                ],
                body: None,
                token: Some("test-token".into()),
            }]
        );
    }

    #[tokio::test]
    async fn error_propagates() {
        let mock = MockClient::default();
        mock.set_list_all_deployments(|| Err(mock_err()));

        let result = deployments::list(
            &mock,
            ListParams {
                activity_status: &[],
                expansions: &[],
                pagination: &Page::default(),
                token: "test-token",
            },
        )
        .await;

        assert!(matches!(result, Err(HTTPErr::MockErr(_))));
    }
}

pub mod list_all {
    use super::*;

    #[tokio::test]
    async fn single_page() {
        let mock = MockClient::default();
        mock.set_list_all_deployments(move || {
            Ok(vec![BackendDeployment {
                id: "dep_1".to_string(),
                ..BackendDeployment::default()
            }])
        });

        let result = deployments::list_all(
            &mock,
            ListAllParams {
                activity_status: &[],
                expansions: &[],
                token: "test-token",
            },
        )
        .await
        .unwrap();

        let expected = vec![BackendDeployment {
            id: "dep_1".to_string(),
            ..BackendDeployment::default()
        }];
        assert_eq!(result, expected);
        assert_eq!(mock.call_count(MockCall::ListDeployments), 1);
        assert_eq!(
            mock.requests(),
            vec![CapturedRequest {
                method: reqwest::Method::GET,
                url: "http://mock/deployments".into(),
                query: vec![
                    ("limit".into(), "100".into()),
                    ("offset".into(), "0".into()),
                ],
                body: None,
                token: Some("test-token".into()),
            }]
        );
    }

    #[tokio::test]
    async fn multi_page_pagination() {
        let mock = MockClient::default();
        let call_num = AtomicUsize::new(0);
        mock.set_list_deployments_page(move || {
            let n = call_num.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Ok(DeploymentList {
                    has_more: true,
                    data: vec![BackendDeployment {
                        id: "dep_1".to_string(),
                        ..BackendDeployment::default()
                    }],
                    ..DeploymentList::default()
                })
            } else {
                Ok(DeploymentList {
                    has_more: false,
                    data: vec![BackendDeployment {
                        id: "dep_2".to_string(),
                        ..BackendDeployment::default()
                    }],
                    ..DeploymentList::default()
                })
            }
        });

        let result = deployments::list_all(
            &mock,
            ListAllParams {
                activity_status: &[],
                expansions: &[],
                token: "test-token",
            },
        )
        .await
        .unwrap();

        let expected = vec![
            BackendDeployment {
                id: "dep_1".to_string(),
                ..BackendDeployment::default()
            },
            BackendDeployment {
                id: "dep_2".to_string(),
                ..BackendDeployment::default()
            },
        ];
        assert_eq!(result, expected);
        assert_eq!(mock.call_count(MockCall::ListDeployments), 2);

        let base_req = CapturedRequest {
            method: reqwest::Method::GET,
            url: "http://mock/deployments".into(),
            query: vec![
                ("limit".into(), "100".into()),
                ("offset".into(), "0".into()),
            ],
            body: None,
            token: Some("test-token".into()),
        };
        assert_eq!(
            mock.requests(),
            vec![
                base_req.clone(),
                CapturedRequest {
                    query: vec![
                        ("limit".into(), "100".into()),
                        ("offset".into(), "100".into()),
                    ],
                    ..base_req
                },
            ]
        );
    }

    #[tokio::test]
    async fn empty_result() {
        let mock = MockClient::default();

        let result = deployments::list_all(
            &mock,
            ListAllParams {
                activity_status: &[],
                expansions: &[],
                token: "test-token",
            },
        )
        .await
        .unwrap();

        let expected: Vec<BackendDeployment> = vec![];
        assert_eq!(result, expected);
        assert_eq!(mock.call_count(MockCall::ListDeployments), 1);
    }

    #[tokio::test]
    async fn error_propagates() {
        let mock = MockClient::default();
        mock.set_list_all_deployments(|| Err(mock_err()));

        let result = deployments::list_all(
            &mock,
            ListAllParams {
                activity_status: &[],
                expansions: &[],
                token: "test-token",
            },
        )
        .await;

        assert!(matches!(result, Err(HTTPErr::MockErr(_))));
    }
}

pub mod update {
    use super::*;

    #[tokio::test]
    async fn success() {
        let mock = MockClient::default();
        mock.set_update_deployment(|| {
            Ok(BackendDeployment {
                id: "dep_1".to_string(),
                ..BackendDeployment::default()
            })
        });

        let updates = UpdateDeploymentRequest {
            activity_status: Some(DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DEPLOYED),
            ..UpdateDeploymentRequest::default()
        };
        let expected_body = serde_json::to_string(&updates).unwrap();

        let result = deployments::update(
            &mock,
            UpdateParams {
                id: "dep_1",
                updates: &updates,
                expansions: &[],
                token: "test-token",
            },
        )
        .await
        .unwrap();

        let expected = BackendDeployment {
            id: "dep_1".to_string(),
            ..BackendDeployment::default()
        };
        assert_eq!(result, expected);
        assert_eq!(mock.call_count(MockCall::UpdateDeployment), 1);
        assert_eq!(
            mock.requests(),
            vec![CapturedRequest {
                method: reqwest::Method::PATCH,
                url: "http://mock/deployments/dep_1".into(),
                query: vec![],
                body: Some(expected_body),
                token: Some("test-token".into()),
            }]
        );
    }

    #[tokio::test]
    async fn with_expansions() {
        let mock = MockClient::default();

        let updates = UpdateDeploymentRequest::default();
        let expected_body = serde_json::to_string(&updates).unwrap();

        deployments::update(
            &mock,
            UpdateParams {
                id: "dep_1",
                updates: &updates,
                expansions: &[DeploymentListExpansion::DEPLOYMENT_LIST_EXPAND_RELEASE],
                token: "test-token",
            },
        )
        .await
        .unwrap();

        assert_eq!(
            mock.requests(),
            vec![CapturedRequest {
                method: reqwest::Method::PATCH,
                url: "http://mock/deployments/dep_1".into(),
                query: vec![("expand".into(), "release".into())],
                body: Some(expected_body),
                token: Some("test-token".into()),
            }]
        );
    }

    #[tokio::test]
    async fn error_propagates() {
        let mock = MockClient::default();
        mock.set_update_deployment(|| Err(mock_err()));

        let updates = UpdateDeploymentRequest::default();
        let result = deployments::update(
            &mock,
            UpdateParams {
                id: "dep_1",
                updates: &updates,
                expansions: &[],
                token: "test-token",
            },
        )
        .await;

        assert!(matches!(result, Err(HTTPErr::MockErr(_))));
    }
}
