// standard library
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::http::mock;
use miru_agent::errors::Error;
use miru_agent::http;
use miru_agent::http::errors::HTTPErr;
use miru_agent::http::request::Params;
use miru_agent::http::ClientI;

// external crates
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;

fn router() -> Router {
    Router::new()
        .route("/ok", get(mock::ok))
        .route("/json", get(mock::json_response))
        .route("/echo", post(mock::echo))
        .route("/not-found", get(mock::not_found))
        .route("/server-error", get(mock::internal_server_error))
        .route("/slow", get(mock::slow))
        .route("/bad-json", get(mock::bad_json))
}

pub mod send {
    use super::*;

    pub mod success {
        use super::*;

        #[tokio::test]
        async fn get_200_returns_success() {
            let server = mock::run_server(router()).await;
            let client = http::Client::new(&server.base_url).unwrap();
            let url = format!("{}/ok", server.base_url);
            let params = Params::get(&url);
            let req = client.build_request(params).unwrap();
            let resp = client.send(req).await.unwrap();
            assert!(resp.reqwest.status().is_success());
        }

        #[tokio::test]
        async fn post_200_with_body() {
            let server = mock::run_server(router()).await;
            let client = http::Client::new(&server.base_url).unwrap();
            let url = format!("{}/echo", server.base_url);
            let params = Params::post(&url, "hello".into());
            let req = client.build_request(params).unwrap();
            let resp = client.send(req).await.unwrap();
            assert!(resp.reqwest.status().is_success());
            let text = resp.reqwest.text().await.unwrap();
            assert_eq!(text, "hello");
        }

        #[tokio::test]
        async fn response_body_matches_mock() {
            let server = mock::run_server(router()).await;
            let client = http::Client::new(&server.base_url).unwrap();
            let url = format!("{}/ok", server.base_url);
            let params = Params::get(&url);
            let req = client.build_request(params).unwrap();
            let resp = client.send(req).await.unwrap();
            let text = resp.reqwest.text().await.unwrap();
            assert_eq!(text, "ok");
        }
    }

    pub mod errors {
        use super::*;

        #[tokio::test]
        async fn connection_refused() {
            let client = http::Client::new("http://127.0.0.1:1").unwrap();
            let params = Params::get("http://127.0.0.1:1/nope");
            let req = client.build_request(params).unwrap();
            let err = client.send(req).await.unwrap_err();
            assert!(matches!(err, HTTPErr::ReqwestErr(_)));
            assert!(err.is_network_connection_error());
        }

        #[tokio::test]
        async fn timeout() {
            let server = mock::run_server(router()).await;
            let client = http::Client::new(&server.base_url).unwrap();
            let url = format!("{}/slow", server.base_url);
            let params = Params::get(&url).with_timeout(Duration::from_millis(50));
            let req = client.build_request(params).unwrap();
            let err = client.send(req).await.unwrap_err();
            assert!(matches!(err, HTTPErr::TimeoutErr { .. }));
        }
    }
}

pub mod execute {
    use super::*;

    #[tokio::test]
    async fn ok_route_returns_body_text() {
        let server = mock::run_server(router()).await;
        let client = http::Client::new(&server.base_url).unwrap();
        let url = format!("{}/ok", server.base_url);
        let params = Params::get(&url);
        let (text, _) = client.execute(params).await.unwrap();
        assert_eq!(text, "ok");
    }

    #[tokio::test]
    async fn not_found_returns_request_failed() {
        let server = mock::run_server(router()).await;
        let client = http::Client::new(&server.base_url).unwrap();
        let url = format!("{}/not-found", server.base_url);
        let params = Params::get(&url);
        let err = client.execute(params).await.unwrap_err();
        assert!(matches!(err, HTTPErr::RequestFailed(_)));
    }

    #[tokio::test]
    async fn server_error_returns_request_failed() {
        let server = mock::run_server(router()).await;
        let client = http::Client::new(&server.base_url).unwrap();
        let url = format!("{}/server-error", server.base_url);
        let params = Params::get(&url);
        let err = client.execute(params).await.unwrap_err();
        assert!(matches!(err, HTTPErr::RequestFailed(_)));
    }
}

pub mod fetch {
    use super::*;

    #[derive(Deserialize, Debug, PartialEq)]
    struct Person {
        name: String,
        age: u32,
    }

    #[tokio::test]
    async fn valid_json_response_deserializes() {
        let server = mock::run_server(router()).await;
        let client = http::Client::new(&server.base_url).unwrap();
        let url = format!("{}/json", server.base_url);
        let params = Params::get(&url);
        let person: Person = http::client::fetch(&client, params).await.unwrap();
        assert_eq!(
            person,
            Person {
                name: "alice".into(),
                age: 30
            }
        );
    }

    #[tokio::test]
    async fn invalid_json_returns_unmarshal_err() {
        let server = mock::run_server(router()).await;
        let client = http::Client::new(&server.base_url).unwrap();
        let url = format!("{}/bad-json", server.base_url);
        let params = Params::get(&url);
        let result: Result<Person, _> = http::client::fetch(&client, params).await;
        assert!(matches!(result, Err(HTTPErr::UnmarshalJSONErr(_))));
    }

    #[tokio::test]
    async fn http_error_returns_request_failed() {
        let server = mock::run_server(router()).await;
        let client = http::Client::new(&server.base_url).unwrap();
        let url = format!("{}/not-found", server.base_url);
        let params = Params::get(&url);
        let result: Result<Person, _> = http::client::fetch(&client, params).await;
        assert!(matches!(result, Err(HTTPErr::RequestFailed(_))));
    }
}

pub mod arc_delegation {
    use super::*;

    #[tokio::test]
    async fn base_url_delegates() {
        let server = mock::run_server(router()).await;
        let client = Arc::new(http::Client::new(&server.base_url).unwrap());
        assert_eq!(client.base_url(), server.base_url);
    }

    #[tokio::test]
    async fn execute_delegates() {
        let server = mock::run_server(router()).await;
        let client = Arc::new(http::Client::new(&server.base_url).unwrap());
        let url = format!("{}/ok", server.base_url);
        let params = Params::get(&url);
        let (text, _) = client.execute(params).await.unwrap();
        assert_eq!(text, "ok");
    }
}
