// internal crates
use crate::errors::Trace;
use crate::models;
use crate::server::responses::config_instance::{ConfigInstanceResponse, ContentField};
use crate::services::errors::ServiceErr;
use crate::storage;

// ========================= ERRORS ======================================= //
#[derive(Debug, thiserror::Error)]
#[error("config instance not found: {id}")]
pub struct ConfigInstanceNotFoundErr {
    pub id: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ConfigInstanceNotFoundErr {
    fn code(&self) -> crate::errors::Code {
        crate::errors::Code::ResourceNotFound
    }
    fn http_status(&self) -> crate::errors::HTTPCode {
        crate::errors::HTTPCode::NOT_FOUND
    }
}

// ========================= SERVICE ====================================== //
pub async fn get(
    cfg_insts: &storage::CfgInsts,
    id: String,
) -> Result<models::ConfigInstance, ServiceErr> {
    let ci = cfg_insts.read_optional(id.clone()).await?;
    match ci {
        Some(ci) => Ok(ci),
        None => Err(ServiceErr::ConfigInstanceNotFoundErr(
            ConfigInstanceNotFoundErr {
                id,
                trace: crate::trace!(),
            },
        )),
    }
}

pub async fn get_response(
    cfg_insts: &storage::CfgInsts,
    cfg_inst_content: &storage::CfgInstContent,
    id: String,
    expand_content: bool,
) -> Result<ConfigInstanceResponse, ServiceErr> {
    let ci = get(cfg_insts, id.clone()).await?;
    let content = if expand_content {
        let raw = cfg_inst_content.read_optional(id).await?;
        raw.map(|data| ContentField {
            format: infer_format(&ci.filepath).to_string(),
            data,
        })
    } else {
        None
    };
    Ok(ConfigInstanceResponse::from_model(&ci, content))
}

pub async fn get_raw_content(
    cfg_insts: &storage::CfgInsts,
    cfg_inst_content: &storage::CfgInstContent,
    id: String,
) -> Result<(models::ConfigInstance, String), ServiceErr> {
    let ci = get(cfg_insts, id.clone()).await?;
    let content = cfg_inst_content.read_optional(id.clone()).await?;
    match content {
        Some(data) => Ok((ci, data)),
        None => Err(ServiceErr::ConfigInstanceNotFoundErr(
            ConfigInstanceNotFoundErr {
                id,
                trace: crate::trace!(),
            },
        )),
    }
}

pub fn infer_format(filepath: &str) -> &'static str {
    if let Some(ext) = filepath.rsplit('.').next() {
        match ext {
            "yaml" | "yml" => "yaml",
            "json" => "json",
            _ => "json",
        }
    } else {
        "json"
    }
}
