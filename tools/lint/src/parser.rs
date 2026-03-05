/// A single `use` statement (possibly spanning multiple lines).
#[derive(Debug, Clone)]
pub struct UseStatement {
    /// Full text including the trailing semicolon and newline(s)
    pub text: String,
    /// 1-based line number where this statement starts
    pub line: usize,
    /// First path segment after `use` (e.g. "std", "crate", "tokio")
    pub root_crate: String,
    /// Full path used for alphabetical sorting (e.g. "crate::app::state::AppState")
    pub sort_key: String,
    /// Any attribute lines preceding this use (e.g. `#[allow(unused_imports)]`)
    pub attrs: Vec<String>,
}

/// An item in the import block.
#[derive(Debug, Clone)]
pub enum ImportBlockItem {
    Comment {
        text: String,
        #[allow(dead_code)] // used for diagnostics and tests
        line: usize,
    },
    Use(UseStatement),
    BlankLine {
        #[allow(dead_code)] // used for diagnostics and tests
        line: usize,
    },
}

/// The parsed import block at the top of a file.
#[derive(Debug)]
pub struct ImportBlock {
    pub items: Vec<ImportBlockItem>,
    /// 1-based line number of the first line AFTER the import block
    pub end_line: usize,
}

impl ImportBlock {
    pub fn use_statements(&self) -> Vec<&UseStatement> {
        self.items
            .iter()
            .filter_map(|item| match item {
                ImportBlockItem::Use(u) => Some(u),
                _ => None,
            })
            .collect()
    }
}

/// Parse the import block from the top of a Rust source file.
pub fn parse(content: &str) -> ImportBlock {
    let lines: Vec<&str> = content.lines().collect();
    let mut items = Vec::new();
    let mut i = 0;
    let mut pending_attrs: Vec<String> = Vec::new();

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // Blank line
        if trimmed.is_empty() {
            // Flush any pending attrs as comments (they weren't followed by a use)
            for attr in pending_attrs.drain(..) {
                items.push(ImportBlockItem::Comment {
                    text: attr,
                    line: i, // approximate
                });
            }
            items.push(ImportBlockItem::BlankLine { line: i + 1 });
            i += 1;
            continue;
        }

        // Comment line (// ...)
        // Only accept group header comments as part of the import block.
        // Any other comment (like `// === section ===`) ends the block.
        if trimmed.starts_with("//") {
            if is_group_header_comment(trimmed) {
                // Flush pending attrs
                for attr in pending_attrs.drain(..) {
                    items.push(ImportBlockItem::Comment {
                        text: attr,
                        line: i,
                    });
                }
                items.push(ImportBlockItem::Comment {
                    text: line.to_string(),
                    line: i + 1,
                });
                i += 1;
                continue;
            } else {
                // Non-header comment — ends the import block
                break;
            }
        }

        // Attribute line (#[...]) — only if followed by a use statement
        // Look ahead to see if a use statement follows (skipping more attrs)
        if trimmed.starts_with("#[") {
            if attrs_lead_to_use(&lines, i) {
                pending_attrs.push(line.to_string());
                i += 1;
                continue;
            } else {
                // Attribute not followed by use — ends the import block
                break;
            }
        }

        // use or pub use statement
        if trimmed.starts_with("use ") || trimmed.starts_with("pub use ") {
            let start_line = i + 1;
            let mut text = String::new();

            // Track brace depth for multi-line use statements
            let mut brace_depth: i32 = 0;
            let first_line = line;
            loop {
                let l = lines[i];
                text.push_str(l);
                text.push('\n');

                for ch in l.chars() {
                    match ch {
                        '{' => brace_depth += 1,
                        '}' => brace_depth -= 1,
                        _ => {}
                    }
                }

                i += 1;

                // Statement ends when we hit a semicolon and brace depth is 0
                if l.trim().ends_with(';') && brace_depth <= 0 {
                    break;
                }
                if i >= lines.len() {
                    break;
                }
            }

            let (root_crate, sort_key) = extract_path_info(first_line);

            let attrs = std::mem::take(&mut pending_attrs);

            items.push(ImportBlockItem::Use(UseStatement {
                text,
                line: start_line,
                root_crate,
                sort_key,
                attrs,
            }));
            continue;
        }

        // Anything else ends the import block
        break;
    }

    // Flush any trailing pending attrs (shouldn't normally happen)
    for attr in pending_attrs.drain(..) {
        items.push(ImportBlockItem::Comment {
            text: attr,
            line: i + 1,
        });
    }

    // Trim trailing blank lines from the block
    while matches!(items.last(), Some(ImportBlockItem::BlankLine { .. })) {
        items.pop();
    }

    ImportBlock {
        items,
        end_line: i + 1,
    }
}

/// Check if a comment line looks like a group header.
fn is_group_header_comment(trimmed: &str) -> bool {
    let t = trimmed.to_lowercase();
    t.starts_with("// standard")
        || t.starts_with("// internal")
        || t.starts_with("// external")
        || t.starts_with("// std ")
}

/// Look ahead from position `start` to see if #[attr] lines lead to a use statement.
fn attrs_lead_to_use(lines: &[&str], start: usize) -> bool {
    let mut j = start;
    while j < lines.len() {
        let t = lines[j].trim();
        if t.starts_with("#[") {
            j += 1;
            continue;
        }
        return t.starts_with("use ") || t.starts_with("pub use ");
    }
    false
}

/// Extract root_crate and sort_key from the first line of a use statement.
fn extract_path_info(line: &str) -> (String, String) {
    let trimmed = line.trim();

    // Strip `pub use ` or `use `
    let path_part = if let Some(rest) = trimmed.strip_prefix("pub use ") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("use ") {
        rest
    } else {
        trimmed
    };

    // The sort key is the full path (without trailing ; or {... content)
    // For "crate::mqtt::{" → sort_key = "crate::mqtt"
    // For "std::sync::Arc;" → sort_key = "std::sync::Arc"
    let sort_key = path_part
        .trim_end_matches(';')
        .trim_end_matches('{')
        .trim_end_matches("::")
        .trim()
        .to_string();

    // root_crate is the first path segment
    let root_crate = sort_key.split("::").next().unwrap_or("").to_string();

    (root_crate, sort_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_line_use() {
        let content = "use std::sync::Arc;\n\nfn main() {}\n";
        let block = parse(content);
        let uses = block.use_statements();
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0].root_crate, "std");
        assert_eq!(uses[0].sort_key, "std::sync::Arc");
    }

    #[test]
    fn parse_multiline_use() {
        let content =
            "use crate::mqtt::{\n    client::Client,\n    device::Device,\n};\n\nfn main() {}\n";
        let block = parse(content);
        let uses = block.use_statements();
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0].root_crate, "crate");
        assert_eq!(uses[0].sort_key, "crate::mqtt");
        assert!(uses[0].text.contains("client::Client"));
    }

    #[test]
    fn parse_with_comments_and_groups() {
        let content = "\
// standard crates
use std::sync::Arc;

// internal crates
use crate::app::state::AppState;

// external crates
use tokio::sync::broadcast;

fn main() {}
";
        let block = parse(content);
        let uses = block.use_statements();
        assert_eq!(uses.len(), 3);
        assert_eq!(uses[0].root_crate, "std");
        assert_eq!(uses[1].root_crate, "crate");
        assert_eq!(uses[2].root_crate, "tokio");
    }

    #[test]
    fn parse_empty_file() {
        let block = parse("");
        assert!(block.items.is_empty());
    }

    #[test]
    fn parse_no_imports() {
        let content = "fn main() {}\n";
        let block = parse(content);
        assert!(block.use_statements().is_empty());
    }

    #[test]
    fn parse_pub_use() {
        let content = "pub use self::errors::SyncErr;\n\nfn main() {}\n";
        let block = parse(content);
        let uses = block.use_statements();
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0].root_crate, "self");
        assert_eq!(uses[0].sort_key, "self::errors::SyncErr");
    }

    #[test]
    fn parse_with_attribute() {
        let content = "#[allow(unused_imports)]\nuse tracing::{debug, error};\n\nfn main() {}\n";
        let block = parse(content);
        let uses = block.use_statements();
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0].attrs.len(), 1);
        assert!(uses[0].attrs[0].contains("allow(unused_imports)"));
    }

    #[test]
    fn stops_at_separator_comment() {
        let content = "\
use crate::models;

// ======== SECTION ======== //
#[derive(Debug)]
pub struct Foo;
";
        let block = parse(content);
        let uses = block.use_statements();
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0].root_crate, "crate");
        // The separator comment and derive should NOT be in the block
        assert_eq!(block.end_line, 3); // block ends before separator
    }

    #[test]
    fn stops_at_derive_without_use() {
        let content = "\
use serde::Serialize;

#[derive(Debug)]
pub enum Foo { A, B }
";
        let block = parse(content);
        let uses = block.use_statements();
        assert_eq!(uses.len(), 1);
    }
}
