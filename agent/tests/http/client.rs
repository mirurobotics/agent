// internal crates
use std::sync::Arc;
use std::time::{Duration, Instant};

// internal crates
use miru_agent::errors::Error;
use miru_agent::http::client::HTTPClient;
use miru_agent::http::errors::HTTPErr;

// external crates
use futures::future::join_all;
use moka::future::Cache;
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};

pub mod headers {
    use super::*;

    #[tokio::test]
    #[serial_test::serial(example_com)]
    async fn validate_headers() {
        let http_client = HTTPClient::new("doesntmatter").await;
        let request = http_client
            .build_get_request("https://example.com/", Duration::from_secs(1), None)
            .unwrap();
        let headers = request.0.headers();
        assert!(headers.contains_key("X-Miru-Agent-Version"));
        assert!(headers.contains_key("X-Host-Name"));
        assert!(headers.contains_key("X-Arch"));
        assert!(headers.contains_key("X-Language"));
        assert!(headers.contains_key("X-OS"));
    }
}

pub mod build_get_request {
    use super::*;

    #[tokio::test]
    #[serial_test::serial(example_com)]
    async fn get_httpbin_org() {
        let http_client = HTTPClient::new("doesntmatter").await;
        let request = http_client
            .build_get_request("https://example.com/", Duration::from_secs(1), None)
            .unwrap();
        let result = http_client.send(request.0, &request.1).await.unwrap();
        assert!(result.status().is_success());
    }
}

pub mod build_post_request {
    use super::*;

    #[tokio::test]
    async fn post_to_postman_echo() {
        let http_client = HTTPClient::new("doesntmatter").await;

        // Create a simple JSON payload
        let payload = serde_json::json!({
            "test": "data",
            "number": 42
        });

        let body = serde_json::to_string(&payload).unwrap();
        let request = http_client
            .build_post_request(
                "https://postman-echo.com/post",
                body,
                Duration::from_secs(10),
                None,
            )
            .unwrap();
        let response = http_client.send(request.0, &request.1).await.unwrap();
        println!("response: {response:?}");
        assert!(response.status().is_success());

        // Parse and verify the response
        let text = response.text().await.unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();

        // httpbin.org echoes back the JSON data in the "json" field
        assert_eq!(json["json"]["test"], "data");
        assert_eq!(json["json"]["number"], 42);
    }
}

pub mod send {
    use super::*;

    pub mod success {
        use super::*;

        #[tokio::test]
        #[serial_test::serial(example_com)]
        async fn get_httpbin_org() {
            let http_client = HTTPClient::new("doesntmatter").await;
            let request = http_client
                .build_get_request("https://httpbin.org/get", Duration::from_secs(10), None)
                .unwrap();
            let result = http_client.send(request.0, &request.1).await.unwrap();
            assert!(result.status().is_success());
        }
    }

    pub mod errors {
        use super::*;

        #[tokio::test]
        async fn network_connection_error() {
            let http_client = HTTPClient::new("doesntmatter").await;
            let request = http_client
                .build_get_request("http://localhost:5454", Duration::from_secs(1), None)
                .unwrap();
            let result = http_client.send(request.0, &request.1).await.unwrap_err();
            assert!(result.is_network_connection_error());
        }

        #[tokio::test]
        #[serial_test::serial(example_dot_com)]
        async fn timeout_error() {
            let http_client = HTTPClient::new("doesntmatter").await;
            let request = http_client
                .build_get_request("https://example.com/", Duration::from_millis(1), None)
                .unwrap();
            let result = http_client.send(request.0, &request.1).await.unwrap_err();
            assert!(matches!(result, HTTPErr::TimeoutErr { .. }));
        }
    }
}

pub mod send_cached {
    use super::*;

    pub mod success {
        use super::*;

        #[tokio::test]
        #[serial_test::serial(example_dot_com)]
        async fn sequential_cache_hit() {
            let http_client = HTTPClient::new("doesntmatter").await;
            let url = "https://example.com/";

            // send the first request
            let start = Instant::now();
            let request = http_client
                .build_get_request(url, Duration::from_secs(1), None)
                .unwrap();
            let is_cache_hit = http_client
                .send_cached(url.to_string(), request.0, &request.1)
                .await
                .unwrap()
                .1;
            assert!(!is_cache_hit);
            let duration = start.elapsed();
            assert!(duration > Duration::from_millis(10));

            // send subsequent requests and check they are cached
            for _ in 0..5 {
                let start = Instant::now();
                let request = http_client
                    .build_get_request(url, Duration::from_secs(1), None)
                    .unwrap();
                let is_cache_hit = http_client
                    .send_cached(url.to_string(), request.0, &request.1)
                    .await
                    .unwrap()
                    .1;
                assert!(is_cache_hit);
                let duration = start.elapsed();
                assert!(duration < Duration::from_millis(300));
            }
        }

        #[tokio::test]
        #[serial_test::serial(example_dot_com)]
        async fn concurrent_cache_hit() {
            let http_client = Arc::new(HTTPClient::new("doesntmatter").await);
            let url = "https://example.com/";

            let start = Instant::now();
            let mut handles = Vec::new();

            // spawn a bunch of concurrent requests
            let num_requests = 50;
            for _ in 0..num_requests {
                let http_client = http_client.clone();
                let url = url.to_string();
                let handle = tokio::spawn(async move {
                    let request = http_client
                        .build_get_request(&url, Duration::from_secs(3), None)
                        .unwrap();
                    http_client
                        .send_cached(url.to_string(), request.0, &request.1)
                        .await
                        .unwrap()
                        .1
                });
                handles.push(handle);
            }

            // Wait for all requests to complete
            let results = join_all(handles).await;

            // should only have one request that is not cached
            let cache_hits = results
                .iter()
                .filter(|r| *r.as_ref().unwrap()) // First unwrap for JoinHandle, second for Result
                .count();
            assert_eq!(cache_hits, num_requests - 1);
            let duration = start.elapsed();
            assert!(duration < Duration::from_millis(500));

            // Verify all requests succeeded
            for result in results {
                result.unwrap(); // Unwrap JoinHandle result
            }
        }

        #[tokio::test]
        async fn errors_not_cached() {
            let http_client = HTTPClient::new("doesntmatter").await;
            let url = "https://httpstat.us/404";

            // send the first request
            let start = Instant::now();
            let request = http_client
                .build_get_request(url, Duration::from_secs(1), None)
                .unwrap();
            http_client
                .send_cached(url.to_string(), request.0, &request.1)
                .await
                .unwrap_err();
            let duration = start.elapsed();
            assert!(duration > Duration::from_millis(10));

            // send subsequent requests and check they are not cached
            for _ in 0..5 {
                let start = Instant::now();
                let request = http_client
                    .build_get_request(url, Duration::from_secs(1), None)
                    .unwrap();
                http_client
                    .send_cached(url.to_string(), request.0, &request.1)
                    .await
                    .unwrap_err();
                let duration = start.elapsed();
                assert!(duration > Duration::from_millis(10));
            }
        }

        #[tokio::test]
        #[serial_test::serial(example_com)]
        async fn cache_expired() {
            let url = "https://example.com/";
            let http_client = HTTPClient::new_with(
                url,
                Duration::from_secs(1),
                Cache::builder()
                    .time_to_live(Duration::from_millis(100))
                    .build(),
            );

            // send the first request
            let start = Instant::now();
            let request = http_client
                .build_get_request(url, Duration::from_secs(1), None)
                .unwrap();
            http_client
                .send_cached(url.to_string(), request.0, &request.1)
                .await
                .unwrap();
            let duration = start.elapsed();
            assert!(duration > Duration::from_millis(10));

            // wait for the cache to expire
            std::thread::sleep(Duration::from_secs(1));

            // send subsequent requests and check they are not cached
            let start = Instant::now();
            let request = http_client
                .build_get_request(url, Duration::from_secs(1), None)
                .unwrap();
            http_client
                .send_cached(url.to_string(), request.0, &request.1)
                .await
                .unwrap();
            let duration = start.elapsed();
            assert!(duration > Duration::from_millis(10));
        }
    }

    pub mod errors {
        use super::*;

        #[tokio::test]
        async fn network_connection_error() {
            let http_client = HTTPClient::new("doesntmatter").await;
            let request = http_client
                .build_get_request("http://localhost:5454", Duration::from_secs(1), None)
                .unwrap();
            let result = http_client
                .send_cached("test".to_string(), request.0, &request.1)
                .await
                .unwrap_err();
            assert!(result.is_network_connection_error());
        }

        #[tokio::test]
        #[serial_test::serial(example_com)]
        async fn timeout_error() {
            let http_client = HTTPClient::new("doesntmatter").await;
            let request = http_client
                .build_get_request("https://example.com/", Duration::from_millis(1), None)
                .unwrap();
            let result = http_client
                .send_cached("test".to_string(), request.0, &request.1)
                .await
                .unwrap_err();
            assert!(matches!(result, HTTPErr::CacheErr { .. }));
        }
    }
}

pub mod handle_response {
    use super::*;

    #[tokio::test]
    async fn endpoint_not_found() {
        // make a request to a non-existent endpoint
        let http_client = HTTPClient::new("doesntmatter").await;
        let request = http_client
            .build_get_request(
                "https://httpbin.org/get/this-page-should-not-exist",
                Duration::from_secs(3),
                None,
            )
            .unwrap();
        let resp = http_client.send(request.0, &request.1).await.unwrap();

        // call the handle_response method
        let response = http_client
            .handle_response(resp, &request.1)
            .await
            .unwrap_err();
        assert!(matches!(response, HTTPErr::RequestFailed { .. }));
    }
}
