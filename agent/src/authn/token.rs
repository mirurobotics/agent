// internal crates
use crate::models::Patch;

// external crates
use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct Token {
    #[serde(with = "secret_string")]
    pub token: SecretString,
    pub expires_at: DateTime<Utc>,
}

/// Serde glue for the `token` field. The bearer token is held in memory as a
/// [`SecretString`] so it can only be read via `expose_secret()`, but it must
/// round-trip through `token.json` as a plain JSON string. This keeps the
/// on-disk contract (`{"token":"<jwt>","expires_at":...}`) unchanged: we expose
/// the secret only to write it and re-wrap it on read.
mod secret_string {
    // external crates
    use secrecy::{ExposeSecret, SecretString};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(secret: &SecretString, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(secret.expose_secret())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<SecretString, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Ok(SecretString::from(raw))
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Token")
            .field("token", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

impl Default for Token {
    fn default() -> Self {
        Self {
            token: SecretString::from(String::new()),
            expires_at: DateTime::<Utc>::default(),
        }
    }
}

impl PartialEq for Token {
    fn eq(&self, other: &Self) -> bool {
        self.expires_at == other.expires_at
            && self.token.expose_secret() == other.token.expose_secret()
    }
}

impl Eq for Token {}

impl Token {
    pub fn is_expired(&self) -> bool {
        self.expires_at < Utc::now()
    }
}

pub struct Updates {
    pub token: Option<SecretString>,
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
