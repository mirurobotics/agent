// internal crates
use crate::models::Patch;

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Token {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Token")
            .field("token", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
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

impl Patch<Updates> for Token {
    fn patch(&mut self, patch: Updates) {
        if let Some(token) = patch.token {
            self.token = token;
        }
        if let Some(expires_at) = patch.expires_at {
            self.expires_at = expires_at;
        }
    }
}
