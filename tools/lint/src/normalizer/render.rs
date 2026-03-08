// internal crates
use crate::normalizer::resolve::ImportNode;
use crate::parser::UseStatement;

pub(crate) fn render_anchor(anchor: &str, tree: &ImportNode) -> String {
    if tree.self_import && tree.renames.is_empty() && !tree.glob && tree.children.is_empty() {
        return format!("use crate::{anchor};\n");
    }

    if !tree.self_import && tree.renames.len() == 1 && !tree.glob && tree.children.is_empty() {
        let alias = tree.renames.iter().next().unwrap_or(&String::new()).clone();
        return format!("use crate::{anchor} as {alias};\n");
    }

    if !tree.self_import && tree.renames.is_empty() && tree.glob && tree.children.is_empty() {
        return format!("use crate::{anchor}::*;\n");
    }

    if !tree.self_import && tree.renames.is_empty() && !tree.glob && tree.children.len() == 1 {
        let (segment, child) = tree
            .children
            .iter()
            .next()
            .unwrap_or_else(|| unreachable!());
        let child_entries = render_child_entries(segment, child);
        if child_entries.len() == 1 {
            return format!("use crate::{anchor}::{};\n", child_entries[0]);
        }
    }

    format!(
        "use crate::{anchor}::{{{}}};\n",
        render_group_parts(tree).join(", ")
    )
}

pub(crate) fn render_original(stmt: &UseStatement) -> String {
    let mut rendered = String::new();
    for attr in &stmt.attrs {
        rendered.push_str(attr);
        rendered.push('\n');
    }
    rendered.push_str(&stmt.text);
    rendered
}

fn render_group_parts(tree: &ImportNode) -> Vec<String> {
    let mut parts = Vec::new();

    if tree.self_import {
        parts.push("self".to_string());
    }
    for alias in &tree.renames {
        parts.push(format!("self as {alias}"));
    }
    if tree.glob {
        parts.push("*".to_string());
    }

    for (segment, child) in &tree.children {
        parts.extend(render_child_entries(segment, child));
    }

    parts
}

fn render_child_entries(segment: &str, node: &ImportNode) -> Vec<String> {
    if node.children.is_empty() && !node.glob {
        let mut parts = Vec::new();
        if node.self_import {
            parts.push(segment.to_string());
        }
        for alias in &node.renames {
            parts.push(format!("{segment} as {alias}"));
        }
        if !parts.is_empty() {
            return parts;
        }
    }

    if !node.self_import && node.renames.is_empty() && node.glob && node.children.is_empty() {
        return vec![format!("{segment}::*")];
    }

    if !node.self_import && node.renames.is_empty() && !node.glob && node.children.len() == 1 {
        let (child_segment, child) = node
            .children
            .iter()
            .next()
            .unwrap_or_else(|| unreachable!());
        let child_entries = render_child_entries(child_segment, child);
        if child_entries.len() == 1 {
            return vec![format!("{segment}::{}", child_entries[0])];
        }
    }

    let mut parts = Vec::new();
    if node.self_import {
        parts.push("self".to_string());
    }
    for alias in &node.renames {
        parts.push(format!("self as {alias}"));
    }
    if node.glob {
        parts.push("*".to_string());
    }
    for (child_segment, child) in &node.children {
        parts.extend(render_child_entries(child_segment, child));
    }

    vec![format!("{segment}::{{{}}}", parts.join(", "))]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normalizer::resolve::{ImportNode, LeafKind};

    #[test]
    fn render_anchor_handles_self_alias_and_glob_special_cases() {
        let mut self_tree = ImportNode::default();
        self_tree.insert(&[], &LeafKind::SelfImport);
        assert_eq!(render_anchor("http", &self_tree), "use crate::http;\n");

        let mut alias_tree = ImportNode::default();
        alias_tree.insert(&[], &LeafKind::Rename("api".to_string()));
        assert_eq!(
            render_anchor("http", &alias_tree),
            "use crate::http as api;\n"
        );

        let mut glob_tree = ImportNode::default();
        glob_tree.insert(&[], &LeafKind::Glob);
        assert_eq!(render_anchor("http", &glob_tree), "use crate::http::*;\n");
    }
}
