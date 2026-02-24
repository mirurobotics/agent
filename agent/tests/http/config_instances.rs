use crate::http::mock::{Call, CapturedRequest, MockClient};
use miru_agent::http::config_instances::{self, GetContentParams};
use miru_agent::http::errors::{HTTPErr, MockErr};

fn mock_err() -> HTTPErr {
    HTTPErr::MockErr(MockErr {
        is_network_conn_err: false,
    })
}

pub mod get_content {
    use super::*;

    #[tokio::test]
    async fn success() {
        let mock = MockClient::default();
        mock.set_get_config_instance_content(|id| {
            assert_eq!(id, "ci_42");
            Ok("config file content here".to_string())
        });

        let result = config_instances::get_content(
            &mock,
            GetContentParams {
                id: "ci_42",
                token: "test-token",
            },
        )
        .await
        .unwrap();

        assert_eq!(result, "config file content here");
        assert_eq!(mock.call_count(Call::GetConfigInstanceContent), 1);
        assert_eq!(
            mock.requests(),
            vec![CapturedRequest {
                call: Call::GetConfigInstanceContent,
                method: reqwest::Method::GET,
                path: "/config_instances/ci_42/content".into(),
                url: "http://mock/config_instances/ci_42/content".into(),
                query: vec![],
                body: None,
                token: Some("test-token".into()),
            }]
        );
    }

    #[tokio::test]
    async fn error_propagates() {
        let mock = MockClient::default();
        mock.set_get_config_instance_content(|_| Err(mock_err()));

        let result = config_instances::get_content(
            &mock,
            GetContentParams {
                id: "ci_99",
                token: "test-token",
            },
        )
        .await;

        assert!(matches!(result, Err(HTTPErr::MockErr(_))));
    }

    #[tokio::test]
    async fn returns_raw_text_not_json() {
        let mock = MockClient::default();
        mock.set_get_config_instance_content(|_| Ok("key: value\nother: 123".to_string()));

        let result = config_instances::get_content(
            &mock,
            GetContentParams {
                id: "ci_yaml",
                token: "tok",
            },
        )
        .await
        .unwrap();

        // Content is returned as-is (raw text, not parsed as JSON)
        assert_eq!(result, "key: value\nother: 123");
    }
}
