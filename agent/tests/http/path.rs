// internal crates
use crate::mocks::http_client::{Call, MockClient};
use backend_api::models::{UpdateDeploymentRequest, UpdateDeviceFromAgentRequest};
use miru_agent::http::{
    config_instances, deployments, devices, file_rules, git_commits, path, releases, uploads,
    HTTPErr,
};

pub mod url {
    use super::*;

    #[test]
    fn joins_base_and_segments() {
        let url = path::url("http://mock/agent/v1", &["uploads", "upl_1", "confirm"]).unwrap();
        assert_eq!(url, "http://mock/agent/v1/uploads/upl_1/confirm");
    }

    #[test]
    fn traversal_stays_in_its_segment() {
        let url = path::url("http://mock/agent/v1", &["git_commits", "../device"]).unwrap();
        assert_eq!(url, "http://mock/agent/v1/git_commits/..%2Fdevice");
    }

    #[test]
    fn rejects_empty_and_dot_segments() {
        for id in ["", ".", ".."] {
            assert!(
                matches!(
                    path::url("http://mock", &["git_commits", id]),
                    Err(HTTPErr::InvalidURLErr(_))
                ),
                "id: {id:?}"
            );
        }
    }

    #[test]
    fn rejects_invalid_base() {
        for base in ["not a url", "mailto:agent@example.com"] {
            assert!(
                matches!(
                    path::url(base, &["git_commits", "gc_1"]),
                    Err(HTTPErr::InvalidURLErr(_))
                ),
                "base: {base}"
            );
        }
    }
}

pub mod request {
    use super::*;

    #[tokio::test]
    async fn getter_sends_encoded_id() {
        let mock = MockClient::default();

        git_commits::get(&mock, "../device", &[], "token")
            .await
            .unwrap();

        let requests = mock.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].call, Call::GetGitCommit);
        assert_eq!(requests[0].path, "/git_commits/..%2Fdevice");
    }

    #[tokio::test]
    async fn getter_rejects_dot_segment_without_request() {
        let mock = MockClient::default();

        let result = git_commits::get(&mock, "..", &[], "token").await;

        assert!(matches!(result, Err(HTTPErr::InvalidURLErr(_))));
        assert!(mock.requests().is_empty());
    }

    #[tokio::test]
    async fn every_id_path_rejects_dot_segment_without_request() {
        let mock = MockClient::default();
        let (id, token) = ("..", "token");
        let dpl_updates = UpdateDeploymentRequest::default();
        let dvc_payload = UpdateDeviceFromAgentRequest::default();

        let errs = [
            git_commits::get(&mock, id, &[], token).await.err(),
            file_rules::get(&mock, id, &[], token).await.err(),
            releases::get(&mock, id, &[], token).await.err(),
            deployments::get(&mock, id, &[], token).await.err(),
            deployments::update(
                &mock,
                deployments::UpdateParams {
                    id,
                    updates: &dpl_updates,
                    token,
                },
            )
            .await
            .err(),
            uploads::vend_credentials(&mock, uploads::VendCredentialsParams { id, token })
                .await
                .err(),
            uploads::confirm(&mock, uploads::ConfirmParams { id, token })
                .await
                .err(),
            devices::update(
                &mock,
                devices::UpdateParams {
                    id,
                    payload: &dvc_payload,
                    token,
                },
            )
            .await
            .err(),
            config_instances::get_content(&mock, config_instances::GetContentParams { id, token })
                .await
                .err(),
        ];

        for (i, err) in errs.iter().enumerate() {
            assert!(
                matches!(err, Some(HTTPErr::InvalidURLErr(_))),
                "call {i}: {err:?}"
            );
        }
        assert!(mock.requests().is_empty());
    }
}
