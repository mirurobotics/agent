use std::time::Duration;

use miru_agent::http::errors::HTTPErr;
use miru_agent::http::request::{self, Headers, Params};

pub mod params {
    use super::*;

    pub mod meta {
        use super::*;

        #[test]
        fn correct_meta_fields() {
            let params = Params::get("https://example.com/api");
            let actual = params.meta().unwrap();
            let expected = request::Meta {
                method: reqwest::Method::GET,
                url: "https://example.com/api".to_string(),
                timeout: Duration::from_secs(10),
            };
            assert_eq!(actual, expected);
        }

        #[test]
        fn meta_with_query_params() {
            let params = Params::get("https://example.com/api")
                .with_query(miru_agent::http::query::QueryParams::new().add("k", "v"));
            let actual = params.meta().unwrap();
            let expected = request::Meta {
                method: reqwest::Method::GET,
                url: "https://example.com/api?k=v".to_string(),
                timeout: Duration::from_secs(10),
            };
            assert_eq!(actual, expected);
        }
    }

    pub mod url_with_query {
        use super::*;

        #[test]
        fn empty_query_returns_url_unchanged() {
            let params = Params::get("https://example.com/api");
            let url = params.url_with_query().unwrap();
            assert_eq!(url, "https://example.com/api");
        }

        #[test]
        fn single_param() {
            let params = Params::get("https://example.com/api");
            let params =
                params.with_query(miru_agent::http::query::QueryParams::new().add("key", "value"));
            let url = params.url_with_query().unwrap();
            assert_eq!(url, "https://example.com/api?key=value");
        }

        #[test]
        fn multiple_params() {
            let params = Params::get("https://example.com/api");
            let params = params.with_query(
                miru_agent::http::query::QueryParams::new()
                    .add("a", "1")
                    .add("b", "2"),
            );
            let url = params.url_with_query().unwrap();
            assert_eq!(url, "https://example.com/api?a=1&b=2");
        }

        #[test]
        fn special_chars_are_percent_encoded() {
            let params = Params::get("https://example.com/api");
            let params = params.with_query(
                miru_agent::http::query::QueryParams::new().add("q", "hello world&more"),
            );
            let url = params.url_with_query().unwrap();
            assert!(url.contains("hello+world"));
            assert!(url.contains("%26more"));
        }

        #[test]
        fn invalid_base_url_returns_error() {
            let params = Params::get("not-a-url");
            let params =
                params.with_query(miru_agent::http::query::QueryParams::new().add("k", "v"));
            let result = params.url_with_query();
            assert!(matches!(result, Err(HTTPErr::InvalidURLErr(_))));
        }
    }

    pub mod constructors {
        use super::*;

        #[test]
        fn get() {
            let actual = Params::get("https://example.com");
            let expected = Params {
                method: reqwest::Method::GET,
                url: "https://example.com",
                query: Vec::new(),
                body: None,
                timeout: Duration::from_secs(10),
                token: None,
            };
            assert_eq!(actual, expected);
        }

        #[test]
        fn post() {
            let actual = Params::post("https://example.com", "body".into());
            let expected = Params {
                method: reqwest::Method::POST,
                url: "https://example.com",
                query: Vec::new(),
                body: Some("body".into()),
                timeout: Duration::from_secs(10),
                token: None,
            };
            assert_eq!(actual, expected);
        }

        #[test]
        fn patch() {
            let actual = Params::patch("https://example.com", "data".into());
            let expected = Params {
                method: reqwest::Method::PATCH,
                url: "https://example.com",
                query: Vec::new(),
                body: Some("data".into()),
                timeout: Duration::from_secs(10),
                token: None,
            };
            assert_eq!(actual, expected);
        }
    }

    pub mod builders {
        use super::*;

        #[test]
        fn with_token_sets_token() {
            let params = Params::get("https://example.com").with_token("my-token");
            assert_eq!(params.token, Some("my-token"));
        }

        #[test]
        fn with_query_sets_pairs() {
            let params = Params::get("https://example.com").with_query(
                miru_agent::http::query::QueryParams::new()
                    .add("a", "1")
                    .add("b", "2"),
            );
            assert_eq!(params.query.len(), 2);
        }

        #[test]
        fn with_timeout_sets_timeout() {
            let params = Params::get("https://example.com").with_timeout(Duration::from_secs(30));
            assert_eq!(params.timeout, Duration::from_secs(30));
        }
    }
}

pub mod meta {
    use super::*;

    #[test]
    fn display() {
        let params =
            Params::get("https://example.com/test").with_timeout(Duration::from_millis(500));
        let meta = params.meta().unwrap();
        let display = format!("{meta}");
        assert_eq!(display, "GET https://example.com/test (timeout: 500ms)");
    }
}

pub mod headers {
    use super::*;

    #[test]
    fn to_map_returns_all_expected_keys() {
        let headers = Headers::default();
        let map = headers.to_map().unwrap();
        assert!(map.contains_key("X-Miru-Agent-Version"));
        assert!(map.contains_key("X-Miru-API-Version"));
        assert!(map.contains_key("X-Host-Name"));
        assert!(map.contains_key("X-Arch"));
        assert!(map.contains_key("X-Language"));
        assert!(map.contains_key("X-OS"));
        assert_eq!(map.len(), 6);
    }
}

pub mod marshal_json {
    use super::*;

    #[test]
    fn serializable_struct_produces_expected_json() {
        #[derive(serde::Serialize)]
        struct Payload {
            key: String,
        }
        let payload = Payload {
            key: "value".into(),
        };
        let json = request::marshal_json(&payload).unwrap();
        assert_eq!(json, r#"{"key":"value"}"#);
    }

    #[test]
    fn unserializable_value_returns_marshal_error() {
        struct AlwaysFails;
        impl serde::Serialize for AlwaysFails {
            fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
                Err(serde::ser::Error::custom("intentional failure"))
            }
        }
        let err = request::marshal_json(&AlwaysFails).unwrap_err();
        assert!(matches!(err, HTTPErr::MarshalJSONErr { .. }));
    }
}

pub mod build {
    use super::*;

    fn make_client() -> reqwest::Client {
        reqwest::Client::new()
    }

    #[test]
    fn get_request_has_correct_method_and_url() {
        let client = make_client();
        let headers = Headers::default();
        let params = Params::get("https://example.com/test");
        let req = request::build(&client, &headers, params).unwrap();
        assert_eq!(req.reqwest.method(), reqwest::Method::GET);
        assert_eq!(req.reqwest.url().as_str(), "https://example.com/test");
    }

    #[test]
    fn post_request_includes_body() {
        let client = make_client();
        let headers = Headers::default();
        let params = Params::post("https://example.com/test", "hello".into());
        let req = request::build(&client, &headers, params).unwrap();
        assert_eq!(req.reqwest.method(), reqwest::Method::POST);
        let body = req.reqwest.body().unwrap().as_bytes().unwrap();
        assert_eq!(body, b"hello");
    }

    #[test]
    fn token_adds_authorization_header() {
        let client = make_client();
        let headers = Headers::default();
        let params = Params::get("https://example.com/test").with_token("tok123");
        let req = request::build(&client, &headers, params).unwrap();
        let auth = req.reqwest.headers().get("authorization").unwrap();
        assert_eq!(auth, "Bearer tok123");
    }

    #[test]
    fn query_params_set_on_request() {
        let client = make_client();
        let headers = Headers::default();
        let params = Params::get("https://example.com/test")
            .with_query(miru_agent::http::query::QueryParams::new().add("k", "v"));
        let req = request::build(&client, &headers, params).unwrap();
        assert!(req.reqwest.url().as_str().contains("k=v"));
    }

    #[test]
    fn get_request_has_all_custom_headers() {
        let client = make_client();
        let headers = Headers::default();
        let params = Params::get("https://example.com/test");
        let req = request::build(&client, &headers, params).unwrap();
        let h = req.reqwest.headers();
        assert!(h.contains_key("X-Miru-Agent-Version"));
        assert!(h.contains_key("X-Miru-API-Version"));
        assert!(h.contains_key("X-Host-Name"));
        assert!(h.contains_key("X-Arch"));
        assert!(h.contains_key("X-Language"));
        assert!(h.contains_key("X-OS"));
    }
}
