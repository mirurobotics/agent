// internal crates
use crate::mocks::upload_client::MockUploadClient;
use backend_api::models::{CreateUploadRequest, UploadSource, UploadStatus};
use miru_agent::http::uploads::{self, ConfirmParams, CreateParams, CredentialsParams};

fn create_request() -> CreateUploadRequest {
    CreateUploadRequest {
        upload_rule_id: "rule_1".to_string(),
        source: Box::new(UploadSource {
            file_path: "/data/a.log".to_string(),
            file_modified_at: "2021-01-01T00:00:00Z".to_string(),
        }),
        digest: "sha256:a".to_string(),
        size: 42,
        incomplete: None,
        release_id: "rls_1".to_string(),
        deployment_id: "dpl_1".to_string(),
    }
}

#[tokio::test]
async fn create_posts_request_and_parses_credentials() {
    let client = MockUploadClient::new();
    let req = create_request();

    let resp = uploads::create(
        &client,
        CreateParams {
            payload: &req,
            token: "tok",
        },
    )
    .await
    .unwrap();

    assert_eq!(resp.upload.id, "upl_1");
    assert_eq!(resp.upload.destination.bucket_name, "my-bucket");
    assert!(resp.credentials.s3_credentials.is_some());

    let captured = client.requests();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].method, reqwest::Method::POST);
    assert_eq!(captured[0].path, "/uploads");
    assert_eq!(captured[0].token.as_deref(), Some("tok"));
    assert!(captured[0].body.is_some());
}

#[tokio::test]
async fn vend_credentials_posts_to_credentials_route() {
    let client = MockUploadClient::new();

    let resp = uploads::vend_credentials(
        &client,
        CredentialsParams {
            upload_id: "upl_1",
            token: "tok",
        },
    )
    .await
    .unwrap();

    assert!(resp.s3_credentials.is_some());
    assert_eq!(client.requests()[0].path, "/uploads/upl_1/credentials");
}

#[tokio::test]
async fn confirm_posts_to_confirm_route() {
    let client = MockUploadClient::new();

    let resp = uploads::confirm(
        &client,
        ConfirmParams {
            upload_id: "upl_1",
            token: "tok",
        },
    )
    .await
    .unwrap();

    assert_eq!(resp.status, UploadStatus::UPLOAD_STATUS_UPLOADED);
    let captured = client.requests();
    assert_eq!(captured[0].method, reqwest::Method::POST);
    assert_eq!(captured[0].path, "/uploads/upl_1/confirm");
    assert_eq!(captured[0].token.as_deref(), Some("tok"));
}
