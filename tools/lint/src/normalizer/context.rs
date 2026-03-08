// standard crates
use std::path::{Component, Path, PathBuf};

#[derive(Debug)]
pub(crate) struct FileContext {
    pub(crate) module_path: Vec<String>,
    pub(crate) is_test_path: bool,
}

impl FileContext {
    pub(crate) fn from_path(file: &Path) -> Self {
        let current_dir = std::env::current_dir().ok();
        Self::from_path_relative_to(current_dir.as_deref(), file)
    }

    pub(crate) fn from_path_relative_to(base_dir: Option<&Path>, file: &Path) -> Self {
        let resolved = resolve_path(base_dir, file);
        Self::from_resolved_path(resolved.as_deref().unwrap_or(file))
    }

    fn from_resolved_path(file: &Path) -> Self {
        let components: Vec<String> = file
            .components()
            .filter_map(|component| match component {
                Component::Normal(value) => value.to_str().map(ToString::to_string),
                _ => None,
            })
            .collect();

        let Some(root_idx) = components
            .iter()
            .rposition(|part| part == "src" || part == "tests")
        else {
            return Self {
                module_path: Vec::new(),
                is_test_path: false,
            };
        };

        let is_test_path = components[root_idx] == "tests";
        let mut module_path = components[root_idx + 1..].to_vec();

        let Some(last) = module_path.last().cloned() else {
            return Self {
                module_path,
                is_test_path,
            };
        };

        let Some(stem) = last.strip_suffix(".rs") else {
            return Self {
                module_path,
                is_test_path,
            };
        };

        match stem {
            "mod" => {
                module_path.pop();
            }
            "lib" | "main" => {
                module_path.clear();
            }
            _ => {
                module_path.pop();
                module_path.push(stem.to_string());
            }
        }

        Self {
            module_path,
            is_test_path,
        }
    }
}

fn resolve_path(base_dir: Option<&Path>, file: &Path) -> Option<PathBuf> {
    let candidate = if file.is_absolute() {
        file.to_path_buf()
    } else {
        base_dir
            .map(|base| base.join(file))
            .unwrap_or_else(|| file.to_path_buf())
    };

    std::fs::canonicalize(&candidate).ok().or(Some(candidate))
}

pub(crate) fn resolve_absolute_path(ctx: &FileContext, path: &[String]) -> Option<Vec<String>> {
    if path.is_empty() {
        return None;
    }

    let mut idx = 0usize;
    let mut absolute = Vec::new();

    match path[0].as_str() {
        "crate" => {
            idx = 1;
        }
        "super" => {
            absolute = ctx.module_path.clone();
            while idx < path.len() && path[idx] == "super" {
                absolute.pop()?;
                idx += 1;
            }
        }
        "self" => {
            absolute = ctx.module_path.clone();
            while idx < path.len() && path[idx] == "self" {
                idx += 1;
            }
        }
        _ => return None,
    }

    absolute.extend(path[idx..].iter().cloned());
    if absolute.is_empty() {
        return None;
    }

    Some(absolute)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_file_context_uses_module_path() {
        let ctx = FileContext::from_path(Path::new("agent/src/http/deployments.rs"));
        assert_eq!(ctx.module_path, vec!["http", "deployments"]);
        assert!(!ctx.is_test_path);
    }

    #[test]
    fn relative_source_file_context_uses_base_dir() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("src/http/deployments.rs");
        let base_dir = dir.path().join("src/http");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "fn main() {}\n").unwrap();

        let ctx = FileContext::from_path_relative_to(
            Some(base_dir.as_path()),
            Path::new("deployments.rs"),
        );
        assert_eq!(ctx.module_path, vec!["http", "deployments"]);
        assert!(!ctx.is_test_path);
    }

    #[test]
    fn mod_file_context_uses_parent_module_path() {
        let ctx = FileContext::from_path(Path::new("agent/src/storage/mod.rs"));
        assert_eq!(ctx.module_path, vec!["storage"]);
        assert!(!ctx.is_test_path);
    }

    #[test]
    fn relative_mod_file_context_uses_parent_module_path() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("src/storage/mod.rs");
        let base_dir = dir.path().join("src/storage");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "pub struct Storage;\n").unwrap();

        let ctx = FileContext::from_path_relative_to(Some(base_dir.as_path()), Path::new("mod.rs"));
        assert_eq!(ctx.module_path, vec!["storage"]);
        assert!(!ctx.is_test_path);
    }

    #[test]
    fn tests_directory_is_marked_test_path() {
        let ctx = FileContext::from_path(Path::new("agent/tests/server/handlers.rs"));
        assert_eq!(ctx.module_path, vec!["server", "handlers"]);
        assert!(ctx.is_test_path);
    }

    #[test]
    fn main_file_context_clears_module_path() {
        let ctx = FileContext::from_path(Path::new("agent/src/main.rs"));
        assert!(ctx.module_path.is_empty());
        assert!(!ctx.is_test_path);
    }

    #[test]
    fn non_source_path_context_is_empty() {
        let ctx = FileContext::from_path(Path::new("README.md"));
        assert!(ctx.module_path.is_empty());
        assert!(!ctx.is_test_path);
    }

    #[test]
    fn resolve_absolute_path_handles_super_chains() {
        let ctx = FileContext {
            module_path: vec!["server".to_string(), "handlers".to_string()],
            is_test_path: false,
        };

        let path = vec![
            "super".to_string(),
            "super".to_string(),
            "errors".to_string(),
            "Trace".to_string(),
        ];

        assert_eq!(
            resolve_absolute_path(&ctx, &path),
            Some(vec!["errors".to_string(), "Trace".to_string()])
        );
    }

    #[test]
    fn resolve_absolute_path_rejects_non_internal_roots() {
        let ctx = FileContext {
            module_path: vec!["http".to_string()],
            is_test_path: false,
        };

        assert_eq!(
            resolve_absolute_path(&ctx, &["tokio".to_string(), "fs".to_string()]),
            None
        );
    }

    #[test]
    fn resolve_absolute_path_rejects_too_many_super_segments() {
        let ctx = FileContext {
            module_path: vec!["http".to_string()],
            is_test_path: false,
        };

        assert_eq!(
            resolve_absolute_path(&ctx, &["super".to_string(), "super".to_string()]),
            None
        );
    }
}
