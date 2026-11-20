pub mod calibration_content_response {
    use axum::body;
    use axum::http::{header, StatusCode};
    use miru_daemon::models::InstanceFormat;
    use miru_daemon::server::response::calibration_content;
    use miru_daemon::services::calibration::Content;
    use uuid::Uuid;

    #[tokio::test]
    async fn sends_the_data_with_the_headers_of_the_format() {
        let report_id = Uuid::new_v4();
        let content = Content {
            report_id,
            format: InstanceFormat::Csv,
            data: "a,b\n".to_string(),
        };

        let response = calibration_content(content);

        assert_eq!(response.status(), StatusCode::OK);
        let headers = response.headers();
        assert_eq!(headers[header::CONTENT_TYPE], "text/csv");
        let disposition = format!("inline; filename=\"{report_id}.csv\"");
        assert_eq!(headers[header::CONTENT_DISPOSITION], disposition);
        let bytes = body::to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(bytes, "a,b\n".as_bytes());
    }

    #[test]
    fn names_text_without_an_extension() {
        let report_id = Uuid::new_v4();
        let content = Content {
            report_id,
            format: InstanceFormat::Text,
            data: String::new(),
        };

        let response = calibration_content(content);

        let disposition = format!("inline; filename=\"{report_id}\"");
        assert_eq!(response.headers()[header::CONTENT_DISPOSITION], disposition);
    }
}
