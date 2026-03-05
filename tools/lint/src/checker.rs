use std::path::Path;

use crate::classifier::{Classifier, ImportGroup};
use crate::config::Config;
use crate::parser::{ImportBlock, ImportBlockItem};

#[derive(Debug)]
pub struct Diagnostic {
    pub line: usize,
    pub kind: String,
    pub message: String,
}

/// Check an import block for violations. Returns a list of diagnostics.
pub fn check(
    file: &Path,
    block: &ImportBlock,
    classifier: &Classifier,
    config: &Config,
) -> Vec<Diagnostic> {
    let _ = file; // used by caller for display
    let uses = block.use_statements();

    if uses.is_empty() {
        return vec![];
    }

    let mut diagnostics = Vec::new();

    // Classify all use statements
    let classified: Vec<(ImportGroup, &crate::parser::UseStatement)> =
        uses.iter().map(|u| (classifier.classify(u), *u)).collect();

    // Build what the "ideal" block looks like and compare
    // 1. Check group ordering: no group should appear after a later group
    let mut last_group: Option<ImportGroup> = None;
    for (group, stmt) in &classified {
        if let Some(prev) = last_group {
            if (*group as u8) < (prev as u8) {
                diagnostics.push(Diagnostic {
                    line: stmt.line,
                    kind: "wrong-group-order".to_string(),
                    message: format!(
                        "{} import `{}` appears after {} imports",
                        group, stmt.sort_key, prev
                    ),
                });
            }
        }
        last_group = Some(*group);
    }

    // 2. Check comment headers
    // Sorting within groups is left to `cargo fmt` (reorder_imports).
    // Walk through items and verify each group has its header
    check_headers(block, &classified, config, &mut diagnostics);

    diagnostics
}

fn check_headers(
    block: &ImportBlock,
    classified: &[(ImportGroup, &crate::parser::UseStatement)],
    config: &Config,
    diagnostics: &mut Vec<Diagnostic>,
) {
    // Determine which groups are present
    let mut groups_present = Vec::new();
    let mut seen = [false; 3];
    for (g, _) in classified {
        let idx = *g as usize;
        if !seen[idx] {
            seen[idx] = true;
            groups_present.push(*g);
        }
    }
    groups_present.sort();

    // For each group present, find its first use statement and check that the
    // preceding comment matches the expected label
    for group in &groups_present {
        let expected_label = match group {
            ImportGroup::Standard => &config.labels.standard,
            ImportGroup::Internal => &config.labels.internal,
            ImportGroup::External => &config.labels.external,
        };

        // Find the first use statement in this group
        let first_stmt = classified.iter().find(|(g, _)| g == group).map(|(_, s)| *s);

        let Some(first) = first_stmt else {
            continue;
        };

        // Look in the block items for the comment before this use,
        // walking backward and skipping blank lines.
        let mut found_header = false;
        for (idx, item) in block.items.iter().enumerate() {
            if let ImportBlockItem::Use(u) = item {
                if u.line == first.line {
                    // Walk backward from idx, skipping blank lines
                    let mut check_idx = idx;
                    while check_idx > 0 {
                        check_idx -= 1;
                        match &block.items[check_idx] {
                            ImportBlockItem::BlankLine { .. } => continue,
                            ImportBlockItem::Comment { text, .. } => {
                                let trimmed = text.trim();
                                if trimmed == expected_label {
                                    found_header = true;
                                } else if is_group_header(trimmed) {
                                    diagnostics.push(Diagnostic {
                                        line: first.line,
                                        kind: "wrong-header".to_string(),
                                        message: format!(
                                            "expected header `{expected_label}`, found `{trimmed}`"
                                        ),
                                    });
                                    found_header = true; // wrong but present
                                }
                                break;
                            }
                            _ => break, // hit a use statement from another group
                        }
                    }
                    break;
                }
            }
        }

        if !found_header {
            diagnostics.push(Diagnostic {
                line: first.line,
                kind: "missing-header".to_string(),
                message: format!("missing `{expected_label}` header before {group} imports"),
            });
        }
    }
}

/// Check if a comment looks like a group header.
fn is_group_header(text: &str) -> bool {
    let t = text.trim().to_lowercase();
    t.starts_with("// standard")
        || t.starts_with("// internal")
        || t.starts_with("// external")
        || t.starts_with("// std")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classifier::Classifier;
    use crate::config::Config;
    use crate::parser::parse;

    fn default_config() -> Config {
        Config {
            internal_crates: vec!["backend_api".to_string()],
            ..Default::default()
        }
    }

    fn check_content(content: &str) -> Vec<Diagnostic> {
        let config = default_config();
        let classifier = Classifier::new(&config);
        let block = parse(content);
        check(Path::new("test.rs"), &block, &classifier, &config)
    }

    #[test]
    fn correct_ordering_no_diagnostics() {
        let content = "\
// standard crates
use std::sync::Arc;

// internal crates
use crate::app::state::AppState;

// external crates
use tokio::sync::broadcast;

fn main() {}
";
        let diags = check_content(content);
        assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");
    }

    #[test]
    fn wrong_group_order() {
        let content = "\
// external crates
use tokio::sync::broadcast;

// internal crates
use crate::app::state::AppState;

fn main() {}
";
        let diags = check_content(content);
        assert!(
            diags.iter().any(|d| d.kind == "wrong-group-order"),
            "expected wrong-group-order diagnostic, got: {diags:?}"
        );
    }

    #[test]
    fn missing_header() {
        let content = "\
use crate::app::state::AppState;
use tokio::sync::broadcast;

fn main() {}
";
        let diags = check_content(content);
        let missing: Vec<_> = diags
            .iter()
            .filter(|d| d.kind == "missing-header")
            .collect();
        assert!(
            missing.len() >= 2,
            "expected at least 2 missing-header diagnostics (internal + external), got: {missing:?}"
        );
    }

    #[test]
    fn wrong_header_label() {
        let content = "\
// standard crates
use crate::app::state::AppState;

fn main() {}
";
        let diags = check_content(content);
        assert!(
            diags.iter().any(|d| d.kind == "wrong-header"),
            "expected wrong-header diagnostic, got: {diags:?}"
        );
    }

    #[test]
    fn header_with_blank_line_between() {
        // Tests the bug fix: blank line between header and use should still find header
        let content = "\
// internal crates

use crate::app::state::AppState;

fn main() {}
";
        let diags = check_content(content);
        let missing: Vec<_> = diags
            .iter()
            .filter(|d| d.kind == "missing-header")
            .collect();
        assert!(
            missing.is_empty(),
            "header should be found despite blank line, got: {missing:?}"
        );
    }

    #[test]
    fn single_group_needs_header() {
        let content = "\
use crate::app::state::AppState;
use crate::http;

fn main() {}
";
        let diags = check_content(content);
        assert!(
            diags.iter().any(|d| d.kind == "missing-header"),
            "expected missing-header for internal imports, got: {diags:?}"
        );
    }

    #[test]
    fn no_imports_no_diagnostics() {
        let content = "fn main() {}\n";
        let diags = check_content(content);
        assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");
    }
}
