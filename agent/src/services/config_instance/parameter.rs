// internal crates
use crate::errors::Trace;
use crate::server::responses::config_instance::{ParameterListResponse, ParameterResponse};
use crate::services::config_instance::get;
use crate::services::errors::ServiceErr;
use crate::storage;

// external crates
use serde_json::Value;

// ========================= ERRORS ======================================= //
#[derive(Debug, thiserror::Error)]
#[error("parameter not found: {key}")]
pub struct ParameterNotFoundErr {
    pub key: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ParameterNotFoundErr {
    fn code(&self) -> crate::errors::Code {
        crate::errors::Code::ResourceNotFound
    }
    fn http_status(&self) -> crate::errors::HTTPCode {
        crate::errors::HTTPCode::NOT_FOUND
    }
}

#[derive(Debug, thiserror::Error)]
#[error("failed to parse config content for config instance {id}: {reason}")]
pub struct ContentParseErr {
    pub id: String,
    pub reason: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ContentParseErr {}

// ========================= SERVICE ====================================== //
pub async fn get_parameter(
    cfg_insts: &storage::CfgInsts,
    cfg_inst_content: &storage::CfgInstContent,
    id: String,
    key: String,
) -> Result<ParameterResponse, ServiceErr> {
    let (ci, raw_content) = get::get_raw_content(cfg_insts, cfg_inst_content, id.clone()).await?;
    let parsed = parse_content(&ci.filepath, &raw_content, &id)?;
    let parts: Vec<&str> = key.split('.').collect();

    let value = navigate(&parsed, &parts).ok_or_else(|| {
        ServiceErr::ParameterNotFoundErr(ParameterNotFoundErr {
            key: key.clone(),
            trace: crate::trace!(),
        })
    })?;

    let key_parts = parts.into_iter().map(|s| s.to_string()).collect();
    Ok(ParameterResponse::new(key_parts, value.clone()))
}

pub async fn list_parameters(
    cfg_insts: &storage::CfgInsts,
    cfg_inst_content: &storage::CfgInstContent,
    id: String,
    prefix: Option<String>,
) -> Result<ParameterListResponse, ServiceErr> {
    let (ci, raw_content) = get::get_raw_content(cfg_insts, cfg_inst_content, id.clone()).await?;
    let parsed = parse_content(&ci.filepath, &raw_content, &id)?;

    let mut params = Vec::new();
    flatten_value(&parsed, &[], &mut params);

    if let Some(ref prefix) = prefix {
        let prefix_parts: Vec<&str> = prefix.split('.').collect();
        params.retain(|p| {
            p.key.len() >= prefix_parts.len()
                && p.key
                    .iter()
                    .zip(prefix_parts.iter())
                    .all(|(a, b)| a == b)
        });
    }

    Ok(ParameterListResponse::new(params))
}

fn parse_content(filepath: &str, raw: &str, id: &str) -> Result<Value, ServiceErr> {
    let format = get::infer_format(filepath);
    match format {
        "yaml" => serde_yml::from_str::<Value>(raw).map_err(|e| {
            ServiceErr::ContentParseErr(ContentParseErr {
                id: id.to_string(),
                reason: e.to_string(),
                trace: crate::trace!(),
            })
        }),
        _ => serde_json::from_str::<Value>(raw).map_err(|e| {
            ServiceErr::ContentParseErr(ContentParseErr {
                id: id.to_string(),
                reason: e.to_string(),
                trace: crate::trace!(),
            })
        }),
    }
}

fn navigate<'a>(value: &'a Value, parts: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for part in parts {
        match current {
            Value::Object(map) => {
                current = map.get(*part)?;
            }
            Value::Array(arr) => {
                let idx: usize = part.parse().ok()?;
                current = arr.get(idx)?;
            }
            _ => return None,
        }
    }
    Some(current)
}

fn flatten_value(value: &Value, prefix: &[String], out: &mut Vec<ParameterResponse>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                let mut new_prefix = prefix.to_vec();
                new_prefix.push(k.clone());
                flatten_value(v, &new_prefix, out);
            }
        }
        Value::Array(arr) => {
            for (i, v) in arr.iter().enumerate() {
                let mut new_prefix = prefix.to_vec();
                new_prefix.push(i.to_string());
                flatten_value(v, &new_prefix, out);
            }
        }
        _ => {
            out.push(ParameterResponse::new(prefix.to_vec(), value.clone()));
        }
    }
}
