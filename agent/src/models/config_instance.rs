// internal crates
use crate::deserialize_error;

// external crates
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde::Serialize;
use tracing::error;
use uuid::Uuid;

// =============================== CONFIG INSTANCE ================================= //
pub type ConfigInstanceID = String;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ConfigInstance {
    pub id: String,
    pub config_type_name: String,
    pub filepath: String,
    pub created_at: DateTime<Utc>,
    pub config_schema_id: String,
    pub config_type_id: String,
}

impl Default for ConfigInstance {
    fn default() -> Self {
        Self {
            id: format!("unknown-{}", Uuid::new_v4()),
            config_type_name: String::new(),
            filepath: format!("unknown-{}", Uuid::new_v4()),
            created_at: DateTime::<Utc>::UNIX_EPOCH,
            config_schema_id: format!("unknown-{}", Uuid::new_v4()),
            config_type_id: format!("unknown-{}", Uuid::new_v4()),
        }
    }
}

impl ConfigInstance {
    pub fn from_backend(
        backend_instance: openapi_client::models::ConfigInstance,
    ) -> ConfigInstance {
        ConfigInstance {
            id: backend_instance.id,
            config_type_name: backend_instance.config_type_name,
            filepath: backend_instance.filepath,
            created_at: backend_instance
                .created_at
                .parse::<DateTime<Utc>>()
                .unwrap_or_else(|e| {
                    error!("Error parsing created_at: {}", e);
                    DateTime::<Utc>::UNIX_EPOCH
                }),
            config_schema_id: backend_instance.config_schema_id,
            config_type_id: backend_instance.config_type_id,
        }
    }
}

impl<'de> Deserialize<'de> for ConfigInstance {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        pub struct DeserializeConfigInstance {
            id: String,
            config_type_name: String,
            filepath: String,
            created_at: Option<DateTime<Utc>>,
            config_schema_id: String,
            config_type_id: String,
        }

        let result = match DeserializeConfigInstance::deserialize(deserializer) {
            Ok(cfg_inst) => cfg_inst,
            Err(e) => {
                error!("Error deserializing config instance: {}", e);
                return Err(e);
            }
        };

        let default = ConfigInstance::default();

        let created_at = result.created_at.unwrap_or_else(|| {
            deserialize_error!("config_instance", "created_at", default.created_at)
        });

        Ok(ConfigInstance {
            id: result.id,
            config_type_name: result.config_type_name,
            filepath: result.filepath,
            created_at,
            config_schema_id: result.config_schema_id,
            config_type_id: result.config_type_id,
        })
    }
}
