// external crates
use crate::models::Mergeable;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Token {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

impl Token {
    pub fn is_expired(&self) -> bool {
        self.expires_at < Utc::now()
    }
}

pub struct Updates {
    pub token: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl Updates {
    pub fn empty() -> Self {
        Self {
            token: None,
            expires_at: None,
        }
    }
}

impl Mergeable<Updates> for Token {
    fn merge(&mut self, updates: Updates) {
        if let Some(token) = updates.token {
            self.token = token;
        }
        if let Some(expires_at) = updates.expires_at {
            self.expires_at = expires_at;
        }
    }
}
