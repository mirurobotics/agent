mod context;
mod render;
mod resolve;

// standard crates
use std::path::Path;

// internal crates
use crate::normalizer::{
    render::{render_anchor, render_original as render_original_impl},
    resolve::{collect_groups, resolve_statements},
};
use crate::parser::UseStatement;

#[derive(Debug)]
pub struct NormalizationDiagnostic {
    pub line: usize,
    pub kind: &'static str,
    pub message: String,
}

#[derive(Debug)]
pub struct RenderedStatement {
    pub line: usize,
    pub text: String,
}

pub fn diagnostics(file: &Path, uses: &[&UseStatement]) -> Vec<NormalizationDiagnostic> {
    let resolutions = resolve_statements(file, uses);
    let mut diagnostics = Vec::new();

    for resolution in &resolutions {
        if resolution.statement.root_crate == "super" && resolution.entries.is_some() {
            diagnostics.push(NormalizationDiagnostic {
                line: resolution.statement.line,
                kind: "relative-super-import",
                message: "prefer absolute `crate::...` imports over `super::...` in source files"
                    .to_string(),
            });
        }

        let Some(entries) = &resolution.entries else {
            continue;
        };

        if entries.len() > 1 {
            diagnostics.push(NormalizationDiagnostic {
                line: resolution.statement.line,
                kind: "multi-anchor-internal-import",
                message:
                    "split grouped `crate::{...}` imports into one statement per top-level anchor"
                        .to_string(),
            });
        }
    }

    let groups = collect_groups(&resolutions);
    for (anchor, group) in groups {
        if group.lines.len() <= 1 {
            continue;
        }

        for line in group.lines {
            diagnostics.push(NormalizationDiagnostic {
                line,
                kind: "split-internal-imports",
                message: format!("merge `crate::{anchor}` imports into a single grouped use"),
            });
        }
    }

    diagnostics.sort_by_key(|diagnostic| diagnostic.line);
    diagnostics
}

pub fn normalize(file: &Path, uses: &[&UseStatement]) -> Vec<RenderedStatement> {
    let mut rendered = Vec::new();
    let resolutions = resolve_statements(file, uses);
    let groups = collect_groups(&resolutions);

    for resolution in &resolutions {
        if resolution.entries.is_none() {
            rendered.push(RenderedStatement {
                line: resolution.statement.line,
                text: render_original_impl(resolution.statement),
            });
        }
    }

    for (anchor, group) in groups {
        if !group.needs_rewrite && group.lines.len() == 1 {
            let stmt = group.originals[0];
            rendered.push(RenderedStatement {
                line: stmt.line,
                text: render_original_impl(stmt),
            });
            continue;
        }

        rendered.push(RenderedStatement {
            line: group.first_line,
            text: render_anchor(&anchor, &group.tree),
        });
    }

    rendered.sort_by_key(|stmt| stmt.line);
    rendered
}

pub fn render_original(stmt: &UseStatement) -> String {
    render_original_impl(stmt)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stmt(line: usize, text: &str, root_crate: &str) -> UseStatement {
        UseStatement {
            text: format!("{text}\n"),
            line,
            root_crate: root_crate.to_string(),
            sort_key: text
                .trim()
                .trim_start_matches("use ")
                .trim_start_matches("pub use ")
                .trim_end_matches(';')
                .to_string(),
            attrs: Vec::new(),
        }
    }

    #[test]
    fn diagnostics_flag_super_and_split_imports() {
        let uses = [
            stmt(1, "use super::errors::HTTPErr;", "super"),
            stmt(2, "use crate::http::request;", "crate"),
        ];
        let refs = uses.iter().collect::<Vec<_>>();

        let diagnostics = diagnostics(Path::new("agent/src/http/deployments.rs"), &refs);
        assert_eq!(diagnostics.len(), 3);
        assert_eq!(diagnostics[0].kind, "relative-super-import");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == "split-internal-imports"));
    }

    #[test]
    fn diagnostics_flag_multi_anchor_internal_imports() {
        let uses = [stmt(
            1,
            "use crate::{concurrent_cache_tests, single_thread_cache_tests};",
            "crate",
        )];
        let refs = uses.iter().collect::<Vec<_>>();

        let diagnostics = diagnostics(Path::new("agent/tests/cache/dir.rs"), &refs);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].kind, "multi-anchor-internal-import");
    }

    #[test]
    fn normalize_merges_split_internal_imports() {
        let uses = [
            stmt(1, "use crate::filesys::dir::Dir;", "crate"),
            stmt(2, "use crate::filesys::path::PathExt;", "crate"),
            stmt(3, "use crate::filesys::{Overwrite, WriteOptions};", "crate"),
        ];
        let refs = uses.iter().collect::<Vec<_>>();

        let normalized = normalize(Path::new("agent/src/filesys/file.rs"), &refs);
        assert_eq!(normalized.len(), 1);
        assert_eq!(
            normalized[0].text,
            "use crate::filesys::{Overwrite, WriteOptions, dir::Dir, path::PathExt};\n"
        );
    }

    #[test]
    fn normalize_rewrites_super_imports() {
        let uses = [
            stmt(1, "use super::errors::HTTPErr;", "super"),
            stmt(2, "use super::{request, ClientI};", "super"),
        ];
        let refs = uses.iter().collect::<Vec<_>>();

        let normalized = normalize(Path::new("agent/src/http/deployments.rs"), &refs);
        assert_eq!(normalized.len(), 1);
        assert_eq!(
            normalized[0].text,
            "use crate::http::{ClientI, errors::HTTPErr, request};\n"
        );
    }

    #[test]
    fn normalize_preserves_aliases_and_self_imports() {
        let uses = [
            stmt(1, "use crate::filesys;", "crate"),
            stmt(2, "use crate::filesys::Overwrite;", "crate"),
            stmt(3, "use crate::services::deployment as dpl_svc;", "crate"),
            stmt(4, "use crate::services::device as dvc_svc;", "crate"),
        ];
        let refs = uses.iter().collect::<Vec<_>>();

        let normalized = normalize(Path::new("agent/src/server/handlers.rs"), &refs);
        assert_eq!(normalized.len(), 2);
        assert_eq!(
            normalized[0].text,
            "use crate::filesys::{self, Overwrite};\n"
        );
        assert_eq!(
            normalized[1].text,
            "use crate::services::{deployment as dpl_svc, device as dvc_svc};\n"
        );
    }

    #[test]
    fn normalize_skips_test_super_imports() {
        let uses = [stmt(1, "use super::mock::Helper;", "super")];
        let refs = uses.iter().collect::<Vec<_>>();

        let diagnostics = diagnostics(Path::new("agent/tests/server/handlers.rs"), &refs);
        let normalized = normalize(Path::new("agent/tests/server/handlers.rs"), &refs);

        assert!(diagnostics.is_empty());
        assert_eq!(normalized[0].text, "use super::mock::Helper;\n");
    }

    #[test]
    fn diagnostics_skip_unrewritable_super_imports() {
        let uses = [stmt(1, "use super::errors::HTTPErr;", "super")];
        let refs = uses.iter().collect::<Vec<_>>();

        let diagnostics = diagnostics(Path::new("deployments.rs"), &refs);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn diagnostics_skip_attributed_super_imports() {
        let uses = [UseStatement {
            text: "use super::errors::HTTPErr;\n".to_string(),
            line: 1,
            root_crate: "super".to_string(),
            sort_key: "super::errors::HTTPErr".to_string(),
            attrs: vec!["#[cfg(feature = \"x\")]".to_string()],
        }];
        let refs = uses.iter().collect::<Vec<_>>();

        let diagnostics = diagnostics(Path::new("agent/src/http/deployments.rs"), &refs);
        let normalized = normalize(Path::new("agent/src/http/deployments.rs"), &refs);

        assert!(diagnostics.is_empty());
        assert_eq!(
            normalized[0].text,
            "#[cfg(feature = \"x\")]\nuse super::errors::HTTPErr;\n"
        );
    }

    #[test]
    fn normalize_preserves_pub_use_statements() {
        let uses = [UseStatement {
            text: "pub use crate::filesys::PathExt;\n".to_string(),
            line: 1,
            root_crate: "crate".to_string(),
            sort_key: "crate::filesys::PathExt".to_string(),
            attrs: Vec::new(),
        }];
        let refs = uses.iter().collect::<Vec<_>>();

        let normalized = normalize(Path::new("agent/src/lib.rs"), &refs);
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].text, "pub use crate::filesys::PathExt;\n");
    }

    #[test]
    fn normalize_preserves_attributed_imports() {
        let uses = [UseStatement {
            text: "use crate::filesys::PathExt;\n".to_string(),
            line: 1,
            root_crate: "crate".to_string(),
            sort_key: "crate::filesys::PathExt".to_string(),
            attrs: vec!["#[cfg(feature = \"test\")]".to_string()],
        }];
        let refs = uses.iter().collect::<Vec<_>>();

        let normalized = normalize(Path::new("agent/src/lib.rs"), &refs);
        assert_eq!(normalized.len(), 1);
        assert_eq!(
            normalized[0].text,
            "#[cfg(feature = \"test\")]\nuse crate::filesys::PathExt;\n"
        );
    }
}
