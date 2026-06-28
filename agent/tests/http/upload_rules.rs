// standard crates
use std::sync::atomic::{AtomicUsize, Ordering};

// internal crates
use crate::mocks::http_client::{Call, CapturedRequest, MockClient};
use backend_api::models::{BaseUploadRule, UploadRuleList};
use miru_agent::http::errors::MockErr;
use miru_agent::http::query::Page;
use miru_agent::http::upload_rules::{self, ListAllParams, ListParams};
use miru_agent::http::HTTPErr;

fn mock_err() -> HTTPErr {
    HTTPErr::MockErr(MockErr {
        is_network_conn_err: false,
    })
}

fn rule(id: &str) -> BaseUploadRule {
    BaseUploadRule {
        id: id.to_string(),
        ..BaseUploadRule::default()
    }
}

pub mod list {
    use super::*;

    #[tokio::test]
    async fn success() {
        let mock = MockClient::default();
        mock.set_list_all_upload_rules(move || Ok(vec![rule("upl_rule_1")]));

        let result = upload_rules::list(
            &mock,
            ListParams {
                pagination: &Page::default(),
                token: "test-token",
            },
        )
        .await
        .unwrap();

        let expected = UploadRuleList {
            total_count: Some(1),
            data: vec![rule("upl_rule_1")],
            ..UploadRuleList::default()
        };
        assert_eq!(result, expected);
        assert_eq!(mock.call_count(Call::ListUploadRules), 1);
        assert_eq!(
            mock.requests(),
            vec![CapturedRequest {
                call: Call::ListUploadRules,
                method: reqwest::Method::GET,
                path: "/upload_rules".into(),
                url: "http://mock/upload_rules".into(),
                query: vec![("limit".into(), "10".into()), ("offset".into(), "0".into())],
                body: None,
                token: Some("test-token".into()),
            }]
        );
    }

    #[tokio::test]
    async fn error_propagates() {
        let mock = MockClient::default();
        mock.set_list_all_upload_rules(|| Err(mock_err()));

        let result = upload_rules::list(
            &mock,
            ListParams {
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
        mock.set_list_all_upload_rules(move || Ok(vec![rule("upl_rule_1")]));

        let result = upload_rules::list_all(
            &mock,
            ListAllParams {
                token: "test-token",
            },
        )
        .await
        .unwrap();

        assert_eq!(result, vec![rule("upl_rule_1")]);
        assert_eq!(mock.call_count(Call::ListUploadRules), 1);
        assert_eq!(
            mock.requests(),
            vec![CapturedRequest {
                call: Call::ListUploadRules,
                method: reqwest::Method::GET,
                path: "/upload_rules".into(),
                url: "http://mock/upload_rules".into(),
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
        mock.set_list_upload_rules_page(move || {
            let n = call_num.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Ok(UploadRuleList {
                    has_more: true,
                    data: vec![rule("upl_rule_1")],
                    ..UploadRuleList::default()
                })
            } else {
                Ok(UploadRuleList {
                    has_more: false,
                    data: vec![rule("upl_rule_2")],
                    ..UploadRuleList::default()
                })
            }
        });

        let result = upload_rules::list_all(
            &mock,
            ListAllParams {
                token: "test-token",
            },
        )
        .await
        .unwrap();

        assert_eq!(result, vec![rule("upl_rule_1"), rule("upl_rule_2")]);
        assert_eq!(mock.call_count(Call::ListUploadRules), 2);

        let base_req = CapturedRequest {
            call: Call::ListUploadRules,
            method: reqwest::Method::GET,
            path: "/upload_rules".into(),
            url: "http://mock/upload_rules".into(),
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

        let result = upload_rules::list_all(
            &mock,
            ListAllParams {
                token: "test-token",
            },
        )
        .await
        .unwrap();

        let expected: Vec<BaseUploadRule> = vec![];
        assert_eq!(result, expected);
        assert_eq!(mock.call_count(Call::ListUploadRules), 1);
    }

    #[tokio::test]
    async fn error_propagates() {
        let mock = MockClient::default();
        mock.set_list_all_upload_rules(|| Err(mock_err()));

        let result = upload_rules::list_all(
            &mock,
            ListAllParams {
                token: "test-token",
            },
        )
        .await;

        assert!(matches!(result, Err(HTTPErr::MockErr(_))));
    }
}
