// internal crates
use miru_agent::filesys::{self, Overwrite};
use miru_agent::models::ConfigInstance;
use miru_agent::services::config_instance as ci_svc;
use miru_agent::services::ServiceErr;
use miru_agent::storage::{CfgInstContent, CfgInsts};

// external crates
use chrono::{DateTime, Utc};
use serde_json::json;

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

async fn seed_json(meta: &CfgInsts, content: &CfgInstContent, id: &str, json_content: &str) {
    let ci = sample_ci(id, "config.json");
    meta.write(id.to_string(), ci, |_, _| false, Overwrite::Allow)
        .await
        .unwrap();
    content
        .write(
            id.to_string(),
            json_content.to_string(),
            |_, _| false,
            Overwrite::Allow,
        )
        .await
        .unwrap();
}

async fn seed_yaml(meta: &CfgInsts, content: &CfgInstContent, id: &str, yaml_content: &str) {
    let ci = sample_ci(id, "config.yaml");
    meta.write(id.to_string(), ci, |_, _| false, Overwrite::Allow)
        .await
        .unwrap();
    content
        .write(
            id.to_string(),
            yaml_content.to_string(),
            |_, _| false,
            Overwrite::Allow,
        )
        .await
        .unwrap();
}

pub mod get_parameter_tests {
    use super::*;

    #[tokio::test]
    async fn returns_top_level_parameter() {
        let (_dir, meta, content) = setup("ci_param_top").await;
        seed_json(&meta, &content, "ci-1", r#"{"timeout_ms": 5000}"#).await;

        let resp = ci_svc::get_parameter(
            &meta,
            &content,
            "ci-1".to_string(),
            "timeout_ms".to_string(),
        )
        .await
        .unwrap();
        assert_eq!(resp.object, "parameter");
        assert_eq!(resp.key, vec!["timeout_ms"]);
        assert_eq!(resp.value, json!(5000));
    }

    #[tokio::test]
    async fn returns_nested_parameter() {
        let (_dir, meta, content) = setup("ci_param_nested").await;
        seed_json(
            &meta,
            &content,
            "ci-1",
            r#"{"network": {"timeout_ms": 3000}}"#,
        )
        .await;

        let resp = ci_svc::get_parameter(
            &meta,
            &content,
            "ci-1".to_string(),
            "network.timeout_ms".to_string(),
        )
        .await
        .unwrap();
        assert_eq!(resp.key, vec!["network", "timeout_ms"]);
        assert_eq!(resp.value, json!(3000));
    }

    #[tokio::test]
    async fn returns_array_element_by_index() {
        let (_dir, meta, content) = setup("ci_param_arr").await;
        seed_json(
            &meta,
            &content,
            "ci-1",
            r#"{"items": ["a", "b", "c"]}"#,
        )
        .await;

        let resp = ci_svc::get_parameter(
            &meta,
            &content,
            "ci-1".to_string(),
            "items.1".to_string(),
        )
        .await
        .unwrap();
        assert_eq!(resp.value, json!("b"));
    }

    #[tokio::test]
    async fn returns_not_found_for_missing_key() {
        let (_dir, meta, content) = setup("ci_param_missing").await;
        seed_json(&meta, &content, "ci-1", r#"{"key": "value"}"#).await;

        let result = ci_svc::get_parameter(
            &meta,
            &content,
            "ci-1".to_string(),
            "nonexistent".to_string(),
        )
        .await;
        assert!(matches!(result, Err(ServiceErr::ParameterNotFoundErr(_))));
    }

    #[tokio::test]
    async fn returns_not_found_for_missing_config_instance() {
        let (_dir, meta, content) = setup("ci_param_no_ci").await;

        let result = ci_svc::get_parameter(
            &meta,
            &content,
            "missing".to_string(),
            "key".to_string(),
        )
        .await;
        assert!(matches!(
            result,
            Err(ServiceErr::ConfigInstanceNotFoundErr(_))
        ));
    }

    #[tokio::test]
    async fn works_with_yaml_content() {
        let (_dir, meta, content) = setup("ci_param_yaml").await;
        seed_yaml(&meta, &content, "ci-1", "network:\n  timeout_ms: 2000\n").await;

        let resp = ci_svc::get_parameter(
            &meta,
            &content,
            "ci-1".to_string(),
            "network.timeout_ms".to_string(),
        )
        .await
        .unwrap();
        assert_eq!(resp.value, json!(2000));
    }
}

pub mod list_parameters_tests {
    use super::*;

    #[tokio::test]
    async fn lists_all_leaf_parameters() {
        let (_dir, meta, content) = setup("ci_list_all").await;
        seed_json(
            &meta,
            &content,
            "ci-1",
            r#"{"a": 1, "b": {"c": 2, "d": 3}}"#,
        )
        .await;

        let resp =
            ci_svc::list_parameters(&meta, &content, "ci-1".to_string(), None)
                .await
                .unwrap();
        assert_eq!(resp.object, "list");
        assert_eq!(resp.data.len(), 3);

        let keys: Vec<Vec<String>> = resp.data.iter().map(|p| p.key.clone()).collect();
        assert!(keys.contains(&vec!["a".to_string()]));
        assert!(keys.contains(&vec!["b".to_string(), "c".to_string()]));
        assert!(keys.contains(&vec!["b".to_string(), "d".to_string()]));
    }

    #[tokio::test]
    async fn filters_by_prefix() {
        let (_dir, meta, content) = setup("ci_list_prefix").await;
        seed_json(
            &meta,
            &content,
            "ci-1",
            r#"{"a": 1, "b": {"c": 2, "d": 3}}"#,
        )
        .await;

        let resp = ci_svc::list_parameters(
            &meta,
            &content,
            "ci-1".to_string(),
            Some("b".to_string()),
        )
        .await
        .unwrap();
        assert_eq!(resp.data.len(), 2);
        for p in &resp.data {
            assert_eq!(p.key[0], "b");
        }
    }

    #[tokio::test]
    async fn handles_arrays_with_numeric_indices() {
        let (_dir, meta, content) = setup("ci_list_arr").await;
        seed_json(
            &meta,
            &content,
            "ci-1",
            r#"{"items": [10, 20]}"#,
        )
        .await;

        let resp =
            ci_svc::list_parameters(&meta, &content, "ci-1".to_string(), None)
                .await
                .unwrap();
        assert_eq!(resp.data.len(), 2);
        assert_eq!(
            resp.data[0].key,
            vec!["items".to_string(), "0".to_string()]
        );
        assert_eq!(resp.data[0].value, json!(10));
        assert_eq!(
            resp.data[1].key,
            vec!["items".to_string(), "1".to_string()]
        );
        assert_eq!(resp.data[1].value, json!(20));
    }

    #[tokio::test]
    async fn returns_empty_for_prefix_with_no_match() {
        let (_dir, meta, content) = setup("ci_list_no_match").await;
        seed_json(&meta, &content, "ci-1", r#"{"a": 1}"#).await;

        let resp = ci_svc::list_parameters(
            &meta,
            &content,
            "ci-1".to_string(),
            Some("z".to_string()),
        )
        .await
        .unwrap();
        assert_eq!(resp.data.len(), 0);
    }

    #[tokio::test]
    async fn returns_not_found_for_missing_config_instance() {
        let (_dir, meta, content) = setup("ci_list_no_ci").await;

        let result =
            ci_svc::list_parameters(&meta, &content, "missing".to_string(), None).await;
        assert!(matches!(
            result,
            Err(ServiceErr::ConfigInstanceNotFoundErr(_))
        ));
    }

    #[tokio::test]
    async fn works_with_yaml_content() {
        let (_dir, meta, content) = setup("ci_list_yaml").await;
        seed_yaml(&meta, &content, "ci-1", "x: 1\ny:\n  z: 2\n").await;

        let resp =
            ci_svc::list_parameters(&meta, &content, "ci-1".to_string(), None)
                .await
                .unwrap();
        assert_eq!(resp.data.len(), 2);
    }
}
