// internal crates
use crate::models;

// external crates
use serde::Serialize;
use serde_json::Value;

// ========================== CONFIG INSTANCE ============================== //
#[derive(Clone, Debug, Serialize)]
pub struct ConfigInstanceResponse {
    pub object: &'static str,
    pub id: String,
    pub config_type_name: String,
    pub filepath: String,
    pub created_at: String,
    pub config_schema_id: String,
    pub config_type_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<ContentField>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ContentField {
    pub format: &'static str,
    pub data: String,
}

impl ConfigInstanceResponse {
    pub fn from_model(ci: &models::ConfigInstance, content: Option<ContentField>) -> Self {
        Self {
            object: "config_instance",
            id: ci.id.clone(),
            config_type_name: ci.config_type_name.clone(),
            filepath: ci.filepath.clone(),
            created_at: ci.created_at.to_rfc3339(),
            config_schema_id: ci.config_schema_id.clone(),
            config_type_id: ci.config_type_id.clone(),
            content,
        }
    }
}

// ============================ PARAMETER ================================== //
#[derive(Clone, Debug, Serialize)]
pub struct ParameterResponse {
    pub object: &'static str,
    pub key: Vec<String>,
    pub value: Value,
}

impl ParameterResponse {
    pub fn new(key: Vec<String>, value: Value) -> Self {
        Self {
            object: "parameter",
            key,
            value,
        }
    }
}

// ========================= PARAMETER LIST ================================ //
#[derive(Clone, Debug, Serialize)]
pub struct ParameterListResponse {
    pub object: &'static str,
    pub data: Vec<ParameterResponse>,
}

impl ParameterListResponse {
    pub fn new(data: Vec<ParameterResponse>) -> Self {
        Self {
            object: "list",
            data,
        }
    }
}
