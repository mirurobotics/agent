// standard crates
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::mocks::http_client as mock;
use miru_agent::errors::Error;
use miru_agent::http::request::Params;
use miru_agent::http::{self, ClientI, HTTPErr};

// external crates
use axum::http::header;
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
        .route("/fixed-date", get(fixed_date))
        .route("/bad-date", get(bad_date))
        .route("/cached-date", get(cached_date))
}

const FIXED_DATE: &str = "Tue, 20 Oct 2026 02:58:04 GMT";

async fn fixed_date() -> ([(header::HeaderName, &'static str); 1], &'static str) {
    ([(header::DATE, FIXED_DATE)], "ok")
}

async fn cached_date() -> ([(header::HeaderName, &'static str); 2], &'static str) {
    ([(header::DATE, FIXED_DATE), (header::AGE, "600")], "ok")
}

async fn bad_date() -> ([(header::HeaderName, &'static str); 1], &'static str) {
    ([(header::DATE, "not a date")], "ok")
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
            assert!(err.is_network_conn_err());
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
    async fn clock_delegates() {
        let server = mock::run_server(router()).await;
        let client = Arc::new(http::Client::new(&server.base_url).unwrap());
        assert_eq!(ClientI::clock(&client).server_now(), None);
        let url = format!("{}/fixed-date", server.base_url);
        let _ = client.execute(Params::get(&url)).await.unwrap();
        assert!(ClientI::clock(&client).server_now().is_some());
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

pub mod clock {
    use super::*;

    // internal crates
    use miru_agent::clock::Clock;

    // external crates
    use chrono::{TimeZone, Utc};

    async fn get(client: &http::Client, base_url: &str, path: &str) {
        let url = format!("{base_url}{path}");
        let req = client.build_request(Params::get(&url)).unwrap();
        let _ = client.send(req).await.unwrap();
    }

    #[tokio::test]
    async fn server_time_is_unknown_before_any_response() {
        let client = http::Client::new("http://127.0.0.1:1").unwrap();
        assert_eq!(client.clock().server_now(), None);
    }

    #[tokio::test]
    async fn a_response_records_the_server_date() {
        let server = mock::run_server(router()).await;
        let client = http::Client::new(&server.base_url).unwrap();
        get(&client, &server.base_url, "/fixed-date").await;
        let server_now = client.clock().server_now().unwrap();
        let expected = Utc.with_ymd_and_hms(2026, 10, 20, 2, 58, 4).unwrap();
        assert!(server_now >= expected && server_now - expected < chrono::TimeDelta::seconds(5));
    }

    #[tokio::test]
    async fn an_error_response_still_records_the_server_date() {
        let server = mock::run_server(router()).await;
        let client = http::Client::new(&server.base_url).unwrap();
        get(&client, &server.base_url, "/not-found").await;
        assert!(client.clock().server_now().is_some());
    }

    #[tokio::test]
    async fn an_unparseable_date_is_ignored() {
        let server = mock::run_server(router()).await;
        let client = http::Client::new(&server.base_url).unwrap();
        get(&client, &server.base_url, "/bad-date").await;
        assert_eq!(client.clock().server_now(), None);
    }

    #[tokio::test]
    async fn a_cached_response_adds_its_age() {
        let server = mock::run_server(router()).await;
        let client = http::Client::new(&server.base_url).unwrap();
        get(&client, &server.base_url, "/cached-date").await;
        let server_now = client.clock().server_now().unwrap();
        let expected = Utc.with_ymd_and_hms(2026, 10, 20, 3, 8, 4).unwrap();
        assert!(server_now >= expected && server_now - expected < chrono::TimeDelta::seconds(5));
    }

    #[tokio::test]
    async fn ignores_the_date_from_a_non_backend_host() {
        let backend = mock::run_server(router()).await;
        let other = mock::run_server(router()).await;
        let client = http::Client::new(&backend.base_url).unwrap();
        get(&client, &other.base_url, "/fixed-date").await;
        assert_eq!(client.clock().server_now(), None);
    }

    #[tokio::test]
    async fn ignores_the_date_when_the_base_url_does_not_parse() {
        let server = mock::run_server(router()).await;
        let client = http::Client::new("not a url").unwrap();
        get(&client, &server.base_url, "/fixed-date").await;
        assert_eq!(client.clock().server_now(), None);
    }

    #[tokio::test]
    async fn with_clock_feeds_the_shared_clock() {
        let server = mock::run_server(router()).await;
        let shared = Clock::new();
        let client = http::Client::with_clock(&server.base_url, shared.clone()).unwrap();
        get(&client, &server.base_url, "/fixed-date").await;
        assert!(shared.server_now().is_some());
    }
}
