use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;
use miru_agent::http;
use miru_agent::http::errors::HTTPErr;
use miru_agent::http::request::Params;
use miru_agent::http::response;
use serde::Deserialize;

use crate::http::mock;

pub mod handle {
    use super::*;

    fn router() -> Router {
        Router::new()
            .route("/ok", get(mock::hello))
            .route("/empty", get(mock::empty))
            .route("/not-found", get(mock::not_found))
            .route("/server-error", get(mock::internal_server_error))
            .route("/unauthorized", get(mock::unauthorized))
    }

    #[tokio::test]
    async fn ok_returns_body_text() {
        let server = mock::run_server(router()).await;
        let client = http::Client::new(&server.base_url).await;
        let url = format!("{}/ok", server.base_url);
        let params = Params::get(&url);
        let req = client.build_request(params).unwrap();
        let resp = client.send(req).await.unwrap();
        let text = response::handle(resp).await.unwrap();
        assert_eq!(text, "hello");
    }

    #[tokio::test]
    async fn empty_body_returns_empty_string() {
        let server = mock::run_server(router()).await;
        let client = http::Client::new(&server.base_url).await;
        let url = format!("{}/empty", server.base_url);
        let params = Params::get(&url);
        let req = client.build_request(params).unwrap();
        let resp = client.send(req).await.unwrap();
        let text = response::handle(resp).await.unwrap();
        assert_eq!(text, "");
    }

    #[tokio::test]
    async fn not_found_returns_request_failed_no_error() {
        let server = mock::run_server(router()).await;
        let client = http::Client::new(&server.base_url).await;
        let url = format!("{}/not-found", server.base_url);
        let params = Params::get(&url);
        let req = client.build_request(params).unwrap();
        let resp = client.send(req).await.unwrap();
        let err = response::handle(resp).await.unwrap_err();
        match &err {
            HTTPErr::RequestFailed(rf) => {
                assert_eq!(rf.status, StatusCode::NOT_FOUND);
                assert!(rf.error.is_none());
            }
            other => panic!("expected RequestFailed, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn unauthorized_returns_request_failed_with_error() {
        let server = mock::run_server(router()).await;
        let client = http::Client::new(&server.base_url).await;
        let url = format!("{}/unauthorized", server.base_url);
        let params = Params::get(&url);
        let req = client.build_request(params).unwrap();
        let resp = client.send(req).await.unwrap();
        let err = response::handle(resp).await.unwrap_err();
        match &err {
            HTTPErr::RequestFailed(rf) => {
                assert_eq!(rf.status, StatusCode::UNAUTHORIZED);
                assert!(rf.error.is_some());
                assert_eq!(rf.error.as_ref().unwrap().error.code, "invalid_jwt_auth");
            }
            other => panic!("expected RequestFailed, got: {other:?}"),
        }
    }
}

pub mod parse_json {
    use super::*;

    fn meta() -> miru_agent::http::request::Meta {
        Params::get("http://test/parse").meta().unwrap()
    }

    #[test]
    fn valid_json_deserializes() {
        #[derive(Deserialize, Debug, PartialEq)]
        struct Data {
            name: String,
        }
        let result: Data = response::parse_json(r#"{"name":"alice"}"#.to_string(), meta()).unwrap();
        assert_eq!(
            result,
            Data {
                name: "alice".into()
            }
        );
    }

    #[test]
    fn invalid_json_returns_error() {
        let result = response::parse_json::<serde_json::Value>("not json{".to_string(), meta());
        assert!(matches!(result, Err(HTTPErr::UnmarshalJSONErr(_))));
    }

    #[test]
    fn wrong_shape_returns_error() {
        #[derive(Deserialize)]
        struct Expected {
            #[allow(dead_code)]
            required_field: String,
        }
        let result = response::parse_json::<Expected>(r#"{"other": 1}"#.to_string(), meta());
        assert!(matches!(result, Err(HTTPErr::UnmarshalJSONErr(_))));
    }

    #[test]
    fn empty_string_returns_error() {
        let result = response::parse_json::<serde_json::Value>(String::new(), meta());
        assert!(matches!(result, Err(HTTPErr::UnmarshalJSONErr(_))));
    }
}
