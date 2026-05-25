// internal crates
use miru_agent::filesys::{self, Overwrite};
use miru_agent::models::ConfigInstance;
use miru_agent::services::config_instance as ci_svc;
use miru_agent::services::ServiceErr;
use miru_agent::storage::{CfgInstContent, CfgInsts};

// external crates
use chrono::{DateTime, Utc};

async fn setup(name: &str) -> (filesys::Dir, CfgInsts, CfgInstContent) {
    let dir = filesys::Dir::create_temp_dir(name).await.unwrap();
    let (meta, _) = CfgInsts::spawn(16, dir.file("cfg_inst_meta.json"), 1000)
        .await
        .unwrap();
    let (content, _) = CfgInstContent::spawn(16, dir.subdir("cfg_inst_content"), 1000)
        .await
        .unwrap();
    (dir, meta, content)
}

fn sample_ci(id: &str, filepath: &str) -> ConfigInstance {
    ConfigInstance {
        id: id.to_string(),
        config_type_name: "robot_config".to_string(),
        filepath: filepath.to_string(),
        created_at: DateTime::<Utc>::UNIX_EPOCH,
        config_schema_id: "schema-1".to_string(),
        config_type_id: "type-1".to_string(),
    }
}

pub mod get_config_instance {
    use super::*;

    #[tokio::test]
    async fn returns_config_instance_by_id() {
        let (_dir, meta, _content) = setup("ci_get_by_id").await;
        let ci = sample_ci("ci-1", "config.json");
        meta.write("ci-1".to_string(), ci.clone(), |_, _| false, Overwrite::Allow)
            .await
            .unwrap();

        let result = ci_svc::get(&meta, "ci-1".to_string()).await.unwrap();
        assert_eq!(result.id, "ci-1");
        assert_eq!(result.config_type_name, "robot_config");
    }

    #[tokio::test]
    async fn returns_not_found_when_missing() {
        let (_dir, meta, _content) = setup("ci_get_not_found").await;

        let result = ci_svc::get(&meta, "nonexistent".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::ConfigInstanceNotFoundErr(_))
        ));
    }
}

pub mod get_response_tests {
    use super::*;

    #[tokio::test]
    async fn returns_response_without_content() {
        let (_dir, meta, content) = setup("ci_resp_no_content").await;
        let ci = sample_ci("ci-1", "config.json");
        meta.write("ci-1".to_string(), ci, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();

        let resp = ci_svc::get_response(&meta, &content, "ci-1".to_string(), false)
            .await
            .unwrap();
        assert_eq!(resp.object, "config_instance");
        assert_eq!(resp.id, "ci-1");
        assert!(resp.content.is_none());
    }

    #[tokio::test]
    async fn returns_response_with_content() {
        let (_dir, meta, content) = setup("ci_resp_with_content").await;
        let ci = sample_ci("ci-1", "config.json");
        meta.write("ci-1".to_string(), ci, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();
        content
            .write(
                "ci-1".to_string(),
                r#"{"key": "value"}"#.to_string(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        let resp = ci_svc::get_response(&meta, &content, "ci-1".to_string(), true)
            .await
            .unwrap();
        assert_eq!(resp.object, "config_instance");
        assert!(resp.content.is_some());
        let cf = resp.content.unwrap();
        assert_eq!(cf.format, "json");
        assert_eq!(cf.data, r#"{"key": "value"}"#);
    }

    #[tokio::test]
    async fn returns_yaml_format_for_yaml_file() {
        let (_dir, meta, content) = setup("ci_resp_yaml").await;
        let ci = sample_ci("ci-1", "config.yaml");
        meta.write("ci-1".to_string(), ci, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();
        content
            .write(
                "ci-1".to_string(),
                "key: value\n".to_string(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        let resp = ci_svc::get_response(&meta, &content, "ci-1".to_string(), true)
            .await
            .unwrap();
        let cf = resp.content.unwrap();
        assert_eq!(cf.format, "yaml");
    }
}

pub mod get_raw_content_tests {
    use super::*;

    #[tokio::test]
    async fn returns_content_when_present() {
        let (_dir, meta, content) = setup("ci_raw_content").await;
        let ci = sample_ci("ci-1", "config.json");
        meta.write("ci-1".to_string(), ci, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();
        content
            .write(
                "ci-1".to_string(),
                r#"{"hello":"world"}"#.to_string(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        let (ci, data) = ci_svc::get_raw_content(&meta, &content, "ci-1".to_string())
            .await
            .unwrap();
        assert_eq!(ci.id, "ci-1");
        assert_eq!(data, r#"{"hello":"world"}"#);
    }

    #[tokio::test]
    async fn returns_not_found_when_meta_missing() {
        let (_dir, meta, content) = setup("ci_raw_no_meta").await;

        let result = ci_svc::get_raw_content(&meta, &content, "missing".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::ConfigInstanceNotFoundErr(_))
        ));
    }

    #[tokio::test]
    async fn returns_not_found_when_content_missing() {
        let (_dir, meta, content) = setup("ci_raw_no_content").await;
        let ci = sample_ci("ci-1", "config.json");
        meta.write("ci-1".to_string(), ci, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();

        let result = ci_svc::get_raw_content(&meta, &content, "ci-1".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::ConfigInstanceNotFoundErr(_))
        ));
    }
}

pub mod infer_format_tests {
    use super::*;

    #[test]
    fn json_extension() {
        assert_eq!(ci_svc::infer_format("config.json"), "json");
    }

    #[test]
    fn yaml_extension() {
        assert_eq!(ci_svc::infer_format("config.yaml"), "yaml");
    }

    #[test]
    fn yml_extension() {
        assert_eq!(ci_svc::infer_format("config.yml"), "yaml");
    }

    #[test]
    fn unknown_extension_defaults_to_json() {
        assert_eq!(ci_svc::infer_format("config.toml"), "json");
    }

    #[test]
    fn no_extension_defaults_to_json() {
        assert_eq!(ci_svc::infer_format("config"), "json");
    }

    #[test]
    fn nested_path_uses_last_extension() {
        assert_eq!(ci_svc::infer_format("/etc/miru/robot.yaml"), "yaml");
    }
}
