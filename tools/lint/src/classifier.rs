use crate::config::Config;
use crate::parser::UseStatement;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ImportGroup {
    Standard = 0,
    Internal = 1,
    External = 2,
}

impl std::fmt::Display for ImportGroup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportGroup::Standard => write!(f, "standard"),
            ImportGroup::Internal => write!(f, "internal"),
            ImportGroup::External => write!(f, "external"),
        }
    }
}

pub struct Classifier {
    internal_crates: Vec<String>,
}

const STANDARD_CRATES: &[&str] = &["std", "core", "alloc"];
const ALWAYS_INTERNAL: &[&str] = &["crate", "super", "self"];

impl Classifier {
    pub fn new(config: &Config) -> Self {
        Self {
            internal_crates: config.internal_crates.clone(),
        }
    }

    pub fn classify(&self, stmt: &UseStatement) -> ImportGroup {
        let root = stmt.root_crate.as_str();

        if STANDARD_CRATES.contains(&root) {
            return ImportGroup::Standard;
        }

        if ALWAYS_INTERNAL.contains(&root) {
            return ImportGroup::Internal;
        }

        if self.internal_crates.iter().any(|c| c == root) {
            return ImportGroup::Internal;
        }

        ImportGroup::External
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::UseStatement;

    fn stmt(root: &str) -> UseStatement {
        UseStatement {
            text: format!("use {root}::something;\n"),
            line: 1,
            root_crate: root.to_string(),
            sort_key: format!("{root}::something"),
            attrs: vec![],
        }
    }

    #[test]
    fn classify_std() {
        let config = Config::default();
        let c = Classifier::new(&config);
        assert_eq!(c.classify(&stmt("std")), ImportGroup::Standard);
        assert_eq!(c.classify(&stmt("core")), ImportGroup::Standard);
        assert_eq!(c.classify(&stmt("alloc")), ImportGroup::Standard);
    }

    #[test]
    fn classify_internal_builtins() {
        let config = Config::default();
        let c = Classifier::new(&config);
        assert_eq!(c.classify(&stmt("crate")), ImportGroup::Internal);
        assert_eq!(c.classify(&stmt("super")), ImportGroup::Internal);
        assert_eq!(c.classify(&stmt("self")), ImportGroup::Internal);
    }

    #[test]
    fn classify_configured_internal() {
        let config = Config {
            internal_crates: vec!["backend_api".to_string(), "device_api".to_string()],
            ..Default::default()
        };
        let c = Classifier::new(&config);
        assert_eq!(c.classify(&stmt("backend_api")), ImportGroup::Internal);
        assert_eq!(c.classify(&stmt("device_api")), ImportGroup::Internal);
    }

    #[test]
    fn classify_external() {
        let config = Config::default();
        let c = Classifier::new(&config);
        assert_eq!(c.classify(&stmt("tokio")), ImportGroup::External);
        assert_eq!(c.classify(&stmt("serde")), ImportGroup::External);
        assert_eq!(c.classify(&stmt("axum")), ImportGroup::External);
    }
}
