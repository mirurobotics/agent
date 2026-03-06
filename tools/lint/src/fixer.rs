// internal crates
use crate::classifier::{Classifier, ImportGroup};
use crate::config::Config;
use crate::parser::ImportBlock;

/// Given file content, its parsed import block, a classifier and config,
/// produce the fixed file content with correctly grouped and labeled imports.
/// Sorting within groups is left to `cargo fmt` (reorder_imports).
pub fn fix_file(
    content: &str,
    block: &ImportBlock,
    classifier: &Classifier,
    config: &Config,
) -> String {
    let uses = block.use_statements();
    if uses.is_empty() {
        return content.to_string();
    }

    // Classify and bucket all use statements
    let mut standard = Vec::new();
    let mut internal = Vec::new();
    let mut external = Vec::new();

    for stmt in &uses {
        let group = classifier.classify(stmt);
        match group {
            ImportGroup::Standard => standard.push(*stmt),
            ImportGroup::Internal => internal.push(*stmt),
            ImportGroup::External => external.push(*stmt),
        }
    }

    // Do NOT sort — preserve original relative order within each group.
    // `cargo fmt` handles sorting via reorder_imports.

    // Build the new import block text
    let mut output = String::new();
    let groups: &[(&[&crate::parser::UseStatement], &str)] = &[
        (&standard, &config.labels.standard),
        (&internal, &config.labels.internal),
        (&external, &config.labels.external),
    ];

    let mut first_group = true;
    for (stmts, label) in groups {
        if stmts.is_empty() {
            continue;
        }

        if !first_group {
            output.push('\n');
        }
        first_group = false;

        output.push_str(label);
        output.push('\n');

        for stmt in *stmts {
            // Write any attributes
            for attr in &stmt.attrs {
                output.push_str(attr);
                output.push('\n');
            }
            output.push_str(&stmt.text);
        }
    }

    // Now splice: replace lines up to end_line with the new block
    let lines: Vec<&str> = content.lines().collect();

    // Find where the import block ends in the original content.
    // block.end_line is 1-based, pointing to the first line AFTER the block.
    // We need to find the byte offset in the original content.
    let block_end_line_idx = block.end_line - 1; // 0-based index

    // Collect the rest of the file (everything after the import block)
    let rest_lines = if block_end_line_idx <= lines.len() {
        &lines[block_end_line_idx..]
    } else {
        &[]
    };

    // Skip leading blank lines from rest to avoid double blank lines
    let rest_skip = rest_lines
        .iter()
        .position(|l| !l.is_empty())
        .unwrap_or(rest_lines.len());
    let rest_lines = &rest_lines[rest_skip..];

    // Ensure a blank line separates the import block from the rest
    if !rest_lines.is_empty() {
        output.push('\n');
    }

    for (i, line) in rest_lines.iter().enumerate() {
        output.push_str(line);
        if i < rest_lines.len() - 1 {
            output.push('\n');
        }
    }

    // Preserve trailing newline if original had one
    if content.ends_with('\n') && !output.ends_with('\n') {
        output.push('\n');
    }

    output
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

    fn fix(content: &str) -> String {
        let config = default_config();
        let classifier = Classifier::new(&config);
        let block = parse(content);
        fix_file(content, &block, &classifier, &config)
    }

    #[test]
    fn already_correct_unchanged() {
        let content = "\
// standard crates
use std::sync::Arc;

// internal crates
use crate::app::state::AppState;

// external crates
use tokio::sync::broadcast;

fn main() {}
";
        assert_eq!(fix(content), content);
    }

    #[test]
    fn adds_missing_headers() {
        let content = "\
use std::sync::Arc;
use crate::app::state::AppState;
use tokio::sync::broadcast;

fn main() {}
";
        let result = fix(content);
        assert!(result.contains("// standard crates\n"));
        assert!(result.contains("// internal crates\n"));
        assert!(result.contains("// external crates\n"));
    }

    #[test]
    fn regroups_misplaced_imports() {
        let content = "\
// external crates
use std::sync::Arc;
use tokio::sync::broadcast;

fn main() {}
";
        let result = fix(content);
        // std::sync::Arc should now be under standard, not external
        assert!(result.starts_with("// standard crates\nuse std::sync::Arc;\n"));
        assert!(result.contains("// external crates\nuse tokio::sync::broadcast;\n"));
    }

    #[test]
    fn preserves_rest_of_file() {
        let content = "\
use std::sync::Arc;

fn main() {
    let x = 42;
}
";
        let result = fix(content);
        assert!(result.contains("fn main() {\n    let x = 42;\n}"));
    }

    #[test]
    fn preserves_trailing_newline() {
        let content = "use std::sync::Arc;\n\nfn main() {}\n";
        let result = fix(content);
        assert!(result.ends_with('\n'), "should preserve trailing newline");
    }

    #[test]
    fn no_double_blank_line() {
        // Content where rest starts with a blank line
        let content = "\
use std::sync::Arc;


fn main() {}
";
        let result = fix(content);
        // Should not have three consecutive newlines (double blank line)
        assert!(
            !result.contains("\n\n\n"),
            "should not have double blank line, got:\n{result}"
        );
    }

    #[test]
    fn handles_attributes_on_use() {
        let content = "\
// external crates
#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

fn main() {}
";
        let result = fix(content);
        assert!(
            result.contains("#[allow(unused_imports)]"),
            "should preserve attribute"
        );
        assert!(
            result.contains("use tracing::{debug, error, info, warn};"),
            "should preserve use statement"
        );
    }

    #[test]
    fn empty_file_unchanged() {
        let content = "";
        assert_eq!(fix(content), content);
    }

    #[test]
    fn single_group_file() {
        let content = "\
use crate::app::state::AppState;
use crate::http;

fn main() {}
";
        let result = fix(content);
        assert!(result.starts_with("// internal crates\n"));
        // Should not have other group headers
        assert!(!result.contains("// standard crates"));
        assert!(!result.contains("// external crates"));
    }
}
