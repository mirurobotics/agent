// standard crates
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

// internal crates
use crate::normalizer::context::{resolve_absolute_path, FileContext};
use crate::parser::UseStatement;

// external crates
use syn::{ItemUse, UseTree};

#[derive(Debug, Clone, Default)]
pub(crate) struct ImportNode {
    pub(crate) self_import: bool,
    pub(crate) glob: bool,
    pub(crate) renames: BTreeSet<String>,
    pub(crate) children: BTreeMap<String, ImportNode>,
}

impl ImportNode {
    pub(crate) fn insert(&mut self, path: &[String], kind: &LeafKind) {
        if path.is_empty() {
            match kind {
                LeafKind::SelfImport => self.self_import = true,
                LeafKind::Glob => self.glob = true,
                LeafKind::Rename(alias) => {
                    self.renames.insert(alias.clone());
                }
            }
            return;
        }

        self.children
            .entry(path[0].clone())
            .or_default()
            .insert(&path[1..], kind);
    }

    pub(crate) fn merge(&mut self, other: ImportNode) {
        self.self_import |= other.self_import;
        self.glob |= other.glob;
        self.renames.extend(other.renames);

        for (segment, child) in other.children {
            self.children.entry(segment).or_default().merge(child);
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum LeafKind {
    SelfImport,
    Glob,
    Rename(String),
}

#[derive(Debug)]
struct FlatLeaf {
    path: Vec<String>,
    kind: LeafKind,
}

#[derive(Debug)]
pub(crate) struct ResolvedEntry {
    pub(crate) anchor: String,
    pub(crate) line: usize,
    pub(crate) needs_rewrite: bool,
    pub(crate) tree: ImportNode,
}

#[derive(Debug)]
pub(crate) struct StatementResolution<'a> {
    pub(crate) statement: &'a UseStatement,
    pub(crate) entries: Option<Vec<ResolvedEntry>>,
}

#[derive(Debug, Default)]
pub(crate) struct AnchorGroup<'a> {
    pub(crate) first_line: usize,
    pub(crate) lines: BTreeSet<usize>,
    pub(crate) needs_rewrite: bool,
    pub(crate) originals: Vec<&'a UseStatement>,
    pub(crate) tree: ImportNode,
}

pub(crate) fn resolve_statements<'a>(
    file: &Path,
    uses: &[&'a UseStatement],
) -> Vec<StatementResolution<'a>> {
    uses.iter()
        .map(|stmt| StatementResolution {
            statement: stmt,
            entries: resolve_entries(file, stmt),
        })
        .collect()
}

pub(crate) fn collect_groups<'a>(
    resolutions: &[StatementResolution<'a>],
) -> BTreeMap<String, AnchorGroup<'a>> {
    let mut groups: BTreeMap<String, AnchorGroup<'a>> = BTreeMap::new();

    for resolution in resolutions {
        let Some(entries) = &resolution.entries else {
            continue;
        };

        for entry in entries {
            let group = groups.entry(entry.anchor.clone()).or_default();
            if group.first_line == 0 {
                group.first_line = entry.line;
            } else {
                group.first_line = group.first_line.min(entry.line);
            }
            group.lines.insert(entry.line);
            group.needs_rewrite |= entry.needs_rewrite;
            group.originals.push(resolution.statement);
            group.tree.merge(entry.tree.clone());
        }
    }

    groups
}

pub(crate) fn resolve_entries(file: &Path, stmt: &UseStatement) -> Option<Vec<ResolvedEntry>> {
    if !stmt.attrs.is_empty() {
        return None;
    }

    let ctx = FileContext::from_path(file);
    if stmt.root_crate == "super" && ctx.is_test_path {
        return None;
    }

    let item = syn::parse_str::<ItemUse>(&stmt.text).ok()?;
    if !matches!(item.vis, syn::Visibility::Inherited) {
        return None;
    }

    let mut flat_leaves = Vec::new();
    let mut prefix = Vec::new();
    flatten_use_tree(&item.tree, &mut prefix, &mut flat_leaves);

    let mut grouped: BTreeMap<String, ImportNode> = BTreeMap::new();
    for leaf in flat_leaves {
        let absolute_path = resolve_absolute_path(&ctx, &leaf.path)?;
        let (anchor, relative) = absolute_path.split_first()?;

        grouped
            .entry(anchor.clone())
            .or_default()
            .insert(relative, &leaf.kind);
    }

    let needs_rewrite = stmt.root_crate == "super" || grouped.len() > 1;
    Some(
        grouped
            .into_iter()
            .map(|(anchor, tree)| ResolvedEntry {
                anchor,
                line: stmt.line,
                needs_rewrite,
                tree,
            })
            .collect(),
    )
}

fn flatten_use_tree(tree: &UseTree, prefix: &mut Vec<String>, out: &mut Vec<FlatLeaf>) {
    match tree {
        UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            flatten_use_tree(&path.tree, prefix, out);
            prefix.pop();
        }
        UseTree::Name(name) => {
            let mut path = prefix.clone();
            path.push(name.ident.to_string());
            out.push(FlatLeaf {
                path,
                kind: LeafKind::SelfImport,
            });
        }
        UseTree::Rename(rename) => {
            let mut path = prefix.clone();
            path.push(rename.ident.to_string());
            out.push(FlatLeaf {
                path,
                kind: LeafKind::Rename(rename.rename.to_string()),
            });
        }
        UseTree::Glob(_) => out.push(FlatLeaf {
            path: prefix.clone(),
            kind: LeafKind::Glob,
        }),
        UseTree::Group(group) => {
            for item in &group.items {
                flatten_use_tree(item, prefix, out);
            }
        }
    }
}
